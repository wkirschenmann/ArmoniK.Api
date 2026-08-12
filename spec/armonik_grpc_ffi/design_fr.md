# DESIGN — Client .NET ArmoniK sur channel gRPC natif Rust

## Introduction

Ce document détaille les décisions techniques de chaque couche de l'architecture définie dans
[SPEC.MD](SPEC.MD), en réponse aux requirements de [requirements.md](requirements.md).

Il fixe les API, types, séquences, contrats d'erreur et les bases du modèle formel TLA+. Les
choix d'implémentation (crates spécifiques, algorithmes internes) restent libres tant qu'ils
respectent les contrats décrits ici.

---

## Couche 1 — `armonik-transport`

### Contrat public : `tower::Service<Uri>`

Le connecteur expose un `tower::Service<Uri, Response = Connection>` où `Connection` implémente
les traits Hyper nécessaires (AsyncRead + AsyncWrite + Connection info).

```rust
/// Le connecteur produit des connexions TCP (+ TLS si configuré) vers une URI.
/// Il ne connaît ni HTTP/2 ni gRPC — c'est un dial réseau.
pub struct TransportConnector { /* ... */ }

impl tower::Service<Uri> for TransportConnector {
    type Response = TransportConnection;
    type Error = TransportError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn call(&mut self, target: Uri) -> Self::Future;
}
```

### Garanties du connecteur

- `poll_ready` ne fait pas d'I/O bloquant (seulement vérification de disponibilité)
- `call` effectue la résolution DNS, le dial TCP, le handshake TLS si configuré
- Le connect timeout s'applique à l'ensemble (DNS + TCP + TLS + proxy CONNECT)
- Une erreur est un `TransportError` structuré (voir ci-dessous)
- Le connecteur est `Clone + Send + Sync` (partageable entre tasks)

### Relation avec le code existant

Le crate `armonik-transport` possède déjà un `TlsConfig` et une `Identity` fonctionnels (serde
flat options, chargement eager du matériel crypto à la lecture de la config). La conception ici
reprend cette approche : on configure ce qu'on veut obtenir (chemins, options), et le matériel
est chargé/validé immédiatement à la construction du connecteur. Les types Rust ci-dessous sont
une évolution du code existant, pas un remplacement from scratch.

### TransportConfig

```rust
pub struct TransportConfig {
    pub endpoint: Endpoint,         // URI cible
    pub tls: Option<TlsConfig>,    // None = plain HTTP
    pub tcp: TcpConfig,            // keepalive, nodelay, etc.
    pub proxy: ProxyConfig,        // disabled | explicit | env | windows_system
    pub connect_timeout: Duration,
}

pub struct TlsConfig {
    pub ca: CaSource,              // System | PemFile(path) | WindowsStore | Insecure
    pub client_identity: Option<IdentitySource>,
    pub override_target_name: Option<String>,
}

/// D'où viennent les racines de confiance (config-time, sérialisable en JSON schema).
pub enum CaSource {
    System,                         // racines OS
    PemFile(PathBuf),              // CA explicite par fichier
    #[cfg(windows)]
    WindowsStore { subject_name: Option<String>, friendly_name: Option<String> },
    Insecure,                       // pas de vérification (opt-in)
}

/// D'où vient l'identité client pour mTLS (config-time, sérialisable en JSON schema).
/// Même pattern que ProxySource : décrit comment obtenir le matériel, pas le matériel lui-même.
pub enum IdentitySource {
    PemFiles { cert: PathBuf, key: PathBuf },
    Pkcs12 { path: PathBuf, password: Option<SecretString> },
    #[cfg(windows)]
    WindowsStore { subject_name: Option<String>, friendly_name: Option<String> },
}

/// Matériel chargé (runtime, après résolution de la source). Non sérialisable.
pub struct Identity {
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl IdentitySource {
    /// Charge le matériel crypto depuis la source configurée.
    pub fn load(&self) -> Result<Identity, ConfigError>;
}

impl CaSource {
    /// Charge les racines de confiance depuis la source configurée.
    /// Retourne None pour System (rustls utilise ses propres racines).
    pub fn load(&self) -> Result<Option<CertificateDer<'static>>, ConfigError>;
}

pub struct ProxyConfig {
    pub source: ProxySource,
    pub credentials: Option<ProxyCredentials>,
}

pub enum ProxySource {
    /// Pas de proxy — connexion directe. Désérialisé depuis "none", "disabled" ou équivalent.
    /// Implique que NO_PROXY est vérifié (si la cible matche NO_PROXY, on reste en direct
    /// même si une autre source est configurée ailleurs). Quand cette variante est choisie
    /// explicitement, NO_PROXY n'intervient pas — c'est un refus inconditionnel.
    None,
    /// Lire le proxy depuis l'environnement (HTTP_PROXY, HTTPS_PROXY, NO_PROXY).
    Environment,
    /// Proxy Windows système (WinHTTP resolver).
    #[cfg(windows)]
    WindowsSystem,
    /// URI propre (sans userinfo) + credentials séparés obligatoires.
    ExplicitWithCredentials { uri: CleanUri, username: String, password: SecretString },
    /// URI brute (peut contenir des credentials dans l'authority, ou pas).
    /// Cas sans auth : URI sans userinfo. Cas avec auth inline : user:pass@host dans l'URI.
    ExplicitUri(Uri),
}

/// URI garantie sans userinfo (pas de user:password@). Validée par construction.
pub struct CleanUri(Uri);

impl CleanUri {
    pub fn new(uri: Uri) -> Result<Self, ConfigError>;
}

pub struct ProxyConfig {
    pub source: ProxySource,
}
```

### TransportError

```rust
pub enum TransportErrorKind {
    DnsResolution,
    TcpConnect,
    TlsHandshake,
    ProxyConnect,
    Timeout,
    Configuration,
}

pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,            // cause chain, sans secrets
}
```

---

## Couche 2 — `armonik-grpc-channel`

### GrpcChannelConfig

```rust
pub struct GrpcChannelConfig {
    pub transport: TransportConfig,
    pub http2: Option<Http2Config>,
    pub retry: Option<RetryConfig>,
    pub default_deadline: Option<Duration>,
    pub pool: Option<PoolConfig>,
    pub user_agent: Option<String>,
    pub eager_connect: bool,    // default = false
}

pub struct Http2Config {
    pub initial_window_size: u32,
    pub max_frame_size: u32,
    pub max_concurrent_streams: Option<u32>,
    pub keepalive_interval: Option<Duration>,
    pub keepalive_timeout: Duration,
}

pub struct RetryConfig {
    pub max_attempts: u32,          // total (initial + retries)
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
    pub retryable_status_codes: Vec<GrpcStatusCode>,
    pub max_buffer_size: usize,     // taille du buffer de rejeu (bytes)
}

pub struct PoolConfig {
    pub idle_timeout: Duration,
    pub max_connections: Option<usize>,
}
```

### GrpcChannel

```rust
pub struct GrpcChannel { /* ... */ }

impl GrpcChannel {
    /// Création. Si eager_connect = true, lance la connexion immédiatement
    /// via l'executor fourni. Sinon, la première connexion est lazy.
    pub fn new(
        config: GrpcChannelConfig,
        executor: impl Executor,
    ) -> Result<Self, ConfigError>;

    /// Démarre une call gRPC.
    pub fn start_call(&self, options: CallStartOptions) -> Result<GrpcCall, ChannelError>;

    /// Ferme le channel : refuse les nouvelles calls, annule les calls actives.
    pub fn close(&self);
}
```

### GrpcCall

```rust
pub struct GrpcCall { /* ... */ }

impl GrpcCall {
    /// Envoie un message (bytes sérialisés protobuf).
    /// En usage Rust pur : copie le buffer et retourne quand accepté par le framing layer.
    /// En usage FFI : le caller garde le buffer valide jusqu'au WRITE_DONE callback.
    /// Un seul send en vol par call (le suivant doit attendre la completion du précédent).
    pub async fn send_message(&self, msg: Bytes) -> Result<(), CallError>;

    /// Signale la fin d'envoi (END_STREAM sur le request body).
    pub async fn end_send(&self) -> Result<(), CallError>;

    /// Récupère les initial metadata (headers HTTP/2 de la réponse).
    /// Bloque jusqu'à réception ou terminal.
    pub async fn recv_initial_metadata(&self) -> Result<Metadata, CallError>;

    /// Récupère le prochain message ou le status terminal.
    /// Chaque appel constitue implicitement une demande d'un message (backpressure naturel).
    /// Retourne End(GrpcStatus) quand le stream est terminé — c'est le terminal de la call.
    pub async fn next_message(&self) -> Result<RecvResult, CallError>;

    /// Annule la call (envoie RST_STREAM).
    pub fn cancel(&self);
}

/// Résultat de la réception : un message ou la fin du stream (status + trailing metadata).
pub enum RecvResult {
    /// Un message gRPC reçu.
    Message(OwnedMessage),
    /// Fin du stream — status gRPC + trailing metadata. Terminal, plus rien après.
    End(GrpcStatus),
}

/// Status terminal d'une call gRPC.
pub struct GrpcStatus {
    pub code: GrpcStatusCode,
    pub message: String,
    pub trailing_metadata: Metadata,
}

/// Message reçu. Owned — le caller libère quand il a fini.
/// En V1, c'est un Vec<u8>. En futur zero-copy, ce sera un handle vers
/// un buffer Rust avec release explicite.
pub struct OwnedMessage {
    pub data: Bytes,
}
```

### CallStartOptions

```rust
pub struct CallStartOptions {
    pub method: String,             // ex: "/armonik.api.grpc.v1.Sessions/CreateSession"
    pub metadata: Metadata,         // request metadata → headers HTTP/2
    pub deadline: Option<Deadline>, // override du default channel
    /// Réservé post-V1 : override de la retry policy pour cet appel.
    /// En V1, doit être None — le default channel s'applique.
    pub reserved_retry: Option<RetryConfig>,
}

/// Deadline absolue ou relative.
pub enum Deadline {
    Absolute(Instant),
    Timeout(Duration),
}
```

Note : la cardinalité n'est pas déclarée au start. Elle est implicite dans l'usage de la call
(un unary fait send_message + end_send + next_message + status ; un server streaming fait
send_message + end_send + next_message en boucle). Le channel n'a pas besoin de la connaître
pour piloter la connexion HTTP/2.

### Executor trait

```rust
pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) -> TaskHandle;
}

/// Handle vers une task spawnée. Permet l'annulation.
pub struct TaskHandle { /* ... */ }
impl TaskHandle {
    pub fn cancel(&self);
}
```

### Consommation par le client Rust ArmoniK

Le client Rust ArmoniK (`armonik::Client<T>`) utilise `armonik-grpc-channel` via un adapter
compatible avec les stubs Tonic générés. L'adapter implémente `tonic::transport::Channel`
(ou le trait `GrpcService<BoxBody>`) en déléguant à `GrpcChannel` :

```rust
/// Adapter qui permet aux stubs Tonic de consommer un GrpcChannel.
pub struct TonicAdapter {
    channel: GrpcChannel,
}

impl tower::Service<http::Request<BoxBody>> for TonicAdapter {
    type Response = http::Response<BoxBody>;
    type Error = tonic::Status;
    type Future = ...;
    // Décode la request Tonic, la route vers GrpcChannel, reconstruit la réponse.
}
```

Le client Rust et le binding .NET partagent le même `GrpcChannel` natif. Le Rust l'utilise
via `TonicAdapter` (pas de FFI). Le .NET l'utilise via la couche FFI + CallInvoker. Les deux
bénéficient du même moteur (retry, deadline, pool, flow control).

### Retry — point de commitment

| Situation | Retryable ? |
|-----------|-------------|
| Unary : erreur avant réponse | Oui |
| Client streaming : erreur, données envoyées ≤ buffer | Oui (rejeu depuis buffer) |
| Client streaming : erreur, données envoyées > buffer | Non (committed) |
| Bidi : erreur, pas de réponse reçue, données ≤ buffer | Oui |
| Bidi : réponse reçue (initial metadata ou message) | Non (committed) |
| Server streaming : erreur avant initial metadata | Oui |
| Deadline restante < backoff | Non |

---

## Couche 3 — `armonik-grpc-channel-ffi`

### Principes

- Le runtime FFI possède un Tokio runtime et l'utilise comme Executor pour le GrpcChannel
- Toute task spawnée est enregistrée dans un task group (joignable au shutdown)
- Les handles sont des identifiants opaques validés dans une registry interne, implémentée
  comme un SlotMap (index + generation, free list chaînée pour allocation O(1))
- Les payloads de messages reçus sont **owned** : le host reçoit un `ak_bytes` qu'il doit
  release. Cela prépare le zero-copy futur (le host pourra désérialiser directement depuis le
  buffer natif avant de release).

### Schéma JSON de configuration

Le schéma JSON est généré depuis `GrpcChannelConfig` + `TransportConfig` + `CallStartOptions`.
Il est la source de vérité pour :
- Les options C# (générées depuis le schéma)
- La documentation des options
- La validation côté Rust à la création du channel et au start de chaque call

Note : `RetryConfig` apparaît à la fois dans `GrpcChannelConfig` (default channel) et dans
`CallStartOptions` (override par call, post-V1). Le schéma couvre les deux usages pour
que le type C# généré soit réutilisable dans les deux contextes.

Le schéma est commité à `packages/rust/armonik-grpc-channel-ffi/include/channel_config.schema.json`.

### Entry points FFI (liste complète V1)

```c
// === Runtime lifecycle ===

// Crée un runtime. Synchrone. Le runtime passe à RUNNING.
// callback + runtime_ctx restent valides jusqu'à AK_EVENT_SHUTDOWN_COMPLETE.
ak_status ak_runtime_create(const ak_runtime_config *config,
                            ak_callback callback,
                            void *runtime_ctx,
                            ak_runtime_handle *out);

// Retourne l'état courant du runtime. Synchrone, non bloquant, thread-safe.
// Le handle reste valide pour cet appel même après RELEASED — c'est la condition
// d'unload. Le host poll jusqu'à AK_RUNTIME_RELEASED avant d'unload la DLL.
ak_runtime_state ak_runtime_status(ak_runtime_handle runtime);

// Déclenche le shutdown. Ferme le start gate, draine/annule les calls.
// Le terminal AK_EVENT_SHUTDOWN_COMPLETE arrive via le callback.
// Idempotent — un second appel est un no-op.
ak_status ak_runtime_begin_shutdown(ak_runtime_handle runtime);

// === Channel ===

// Crée un channel depuis un JSON de config. Synchrone (pas d'I/O sauf si eager).
// Lifecycle : libéré par ak_channel_release.
ak_status ak_channel_create(ak_runtime_handle runtime,
                            ak_bytes_in config_json,
                            ak_channel_handle *out);

// Libère le channel. Les calls en cours sont annulées (CANCELLED).
// Le handle n'est plus valide après cet appel.
void ak_channel_release(ak_channel_handle channel);

// === Call ===

// Démarre une call gRPC. call_ctx est retourné dans chaque callback de cette call.
// Le host DOIT allouer call_ctx avant cet appel et le garder valide jusqu'au terminal.
// Si le start échoue (retour != AK_OK), aucun callback ne sera émis pour ce call_ctx.
ak_status ak_call_start(ak_channel_handle channel,
                        const ak_call_start_options *options,
                        void *call_ctx,
                        ak_call_handle *out);

// Envoie un message (zero-copy). Rust lit directement les bytes pointés par message.
// Le host DOIT garder le buffer pointé valide (pinned) jusqu'à réception du callback
// AK_EVENT_WRITE_DONE pour cette call. Un seul send en vol par call à la fois :
// le host ne doit pas rappeler send_message avant d'avoir reçu WRITE_DONE.
// Le terminal STATUS peut arriver à la place de WRITE_DONE (erreur/cancel).
ak_status ak_call_send_message(ak_call_handle call, ak_bytes_in message);

// Signale la fin d'envoi (END_STREAM). Plus de send_message après.
ak_status ak_call_end_send(ak_call_handle call);

// Demande le prochain message. Produit un callback MESSAGE ou STATUS.
// Un seul peut être en vol à la fois.
ak_status ak_call_request_next_message(ak_call_handle call);

// Annule la call. Produit un callback STATUS avec code CANCELLED.
// Idempotent — un second appel est un no-op.
ak_status ak_call_cancel(ak_call_handle call);

// Libère le handle. Annule la call si pas déjà terminée.
// Le terminal callback arrive quand même après le release.
void ak_call_release(ak_call_handle call);

// === Utilitaires ===

// Version de l'ABI. À comparer avec AK_ABI_VERSION compilé dans le binding.
int ak_abi_version(void);

// Signale que le host a consommé le payload d'un event. Double sémantique :
// 1. Libère la mémoire native (Rust dealloc le buffer)
// 2. Arme la réception du prochain event de cette call (demand signal)
// Un seul event non-consumed par call à la fois — tant que le host n'a pas
// appelé ak_event_consumed, le runtime ne délivre pas le suivant.
void ak_event_consumed(ak_bytes payload);
```

### Surface ABI détaillée

```c
// === Types opaques ===
// Lifecycle : créé par ak_runtime_create, libéré implicitement à AK_RELEASED.
typedef struct ak_runtime_s *ak_runtime_handle;

// Lifecycle : créé par ak_channel_create, libéré par ak_channel_release.
// Les calls en cours sont annulées. Les ressources internes survivent
// jusqu'à quiescence des calls puis sont libérées par le runtime.
typedef struct ak_channel_s *ak_channel_handle;

// Lifecycle : créé par ak_call_start, libéré par ak_call_release.
// Le release demande une cancellation mais le terminal callback arrive
// quand même — le handle est invalide pour de nouveaux downcalls après release.
typedef struct ak_call_s    *ak_call_handle;

// Token choisi par le host, passé à ak_call_start, retourné dans chaque callback
// de cette call. C'est un void* opaque — Rust ne le déréférence jamais.
// Le host y met ce qu'il veut : GCHandle (.NET), GlobalRef (Java), id (Python).
// Pas de lifecycle natif — le host gère l'objet pointé.
// Doit rester valide jusqu'à réception du terminal (AK_EVENT_STATUS) de la call.
typedef void *ak_call_ctx;

// === Buffers ===

// Emprunté par Rust pendant le downcall uniquement. Le host reste owner.
// Pas de lifecycle natif — le host gère la mémoire pointée.
typedef struct {
    const uint8_t *ptr;
    size_t len;
} ak_bytes_in;

// Owned par le host après réception. Le host DOIT appeler ak_event_consumed
// exactement une fois quand il a fini de consommer les données.
//
// owner : handle opaque vers l'allocation Rust sous-jacente. Le ptr/len
// est une vue en lecture seule sur des bytes qui peuvent être un sous-ensemble
// d'une allocation plus grande (ex: un Arc<Vec<u8>>). C'est owner qui
// identifie ce qu'il faut libérer — ptr seul ne suffit pas car il peut
// pointer au milieu d'une allocation reference-counted. Le host passe
// owner inchangé à ak_event_consumed.
typedef struct {
    const uint8_t *ptr;     // vue lecture seule
    size_t len;             // nombre de bytes lisibles à ptr
    void *owner;            // opaque — passé tel quel à ak_bytes_release
} ak_bytes;

// === Events ===
typedef enum {
    AK_RUNTIME_RUNNING           = 1,  // opérationnel, accepte channels et calls
    AK_RUNTIME_STOPPING          = 2,  // start gate fermé, channels en fermeture
    AK_RUNTIME_DRAINING          = 3,  // attend quiescence des tasks et callbacks
    AK_RUNTIME_RELEASED          = 4,  // quiescent, unload autorisé
    AK_RUNTIME_FAILED_UNQUIESCED = 5,  // quiescence impossible, unload interdit
} ak_runtime_state;
// NOT_INITIALIZED n'est pas un état observable : avant ak_runtime_create réussi,
// le host n'a pas de handle. Après RELEASED, le handle est uniquement valide
// pour ak_runtime_status (qui continue de retourner RELEASED).

typedef enum {
    AK_EVENT_INITIAL_METADATA  = 1,  // payload = metadata blob (owned)
    AK_EVENT_MESSAGE           = 2,  // payload = message bytes (owned)
    AK_EVENT_STATUS            = 3,  // terminal — payload = status + trailing metadata (owned)
    AK_EVENT_SHUTDOWN_COMPLETE = 4,  // runtime terminal
    AK_EVENT_WRITE_DONE        = 5,  // le buffer send a été consommé par le réseau, host peut unpin
} ak_event_kind;

// Passé sur la pile dans le callback — pas de lifecycle propre.
// Le champ payload est owned et doit être releasé par le host.
typedef struct {
    ak_event_kind kind;
    ak_bytes payload;               // owned — host doit appeler ak_event_consumed
    int32_t status_code;            // grpc status (pertinent seulement pour AK_EVENT_STATUS)
} ak_event;

// === Callback ===
// Lifecycle du function pointer : doit rester valide pour la durée de vie du runtime.
// Lifecycle du runtime_ctx : géré par le host (GCHandle), doit rester valide jusqu'à
// réception de AK_EVENT_SHUTDOWN_COMPLETE.
typedef void (*ak_callback)(
    void *runtime_ctx,
    void *call_ctx,
    const ak_event *event);
// Le callback reçoit un event dont le payload est owned.
// Le host DOIT appeler ak_event_consumed sur event->payload
// après avoir consommé les données. Cet appel libère la mémoire ET arme le suivant.
// Le callback est sérialisé par call, concurrent entre calls.
```

### Séquence d'un unary call

```text
Host (C#)                          FFI (Rust)
─────────                          ──────────
callState = new CallState(...)
gcHandle = GCHandle.Alloc(callState)
ak_call_start(channel, opts,       → valide channel, crée GrpcCall,
              gcHandle, &handle)      enregistre dans task group
                                     retourne handle
pin(msg_buffer)                    // pin le buffer managé
ak_call_send_message(handle, msg)  → Rust lit directement msg (zero-copy send)
                                   ... réseau : Rust envoie sur HTTP/2 ...
                          callback(runtime_ctx, gcHandle, &evt_w) ←
                            evt_w.kind = WRITE_DONE     [buffer consommé]
unpin(msg_buffer)                  // safe : Rust a fini de lire
ak_call_end_send(handle)           → signal end_send
                                   ... réseau ...
                          callback(runtime_ctx, gcHandle, &evt1) ←
                            evt1.kind = INITIAL_METADATA  [auto, avant tout message]
                            evt1.payload = ak_bytes{ptr, len, owner}
ak_event_consumed(evt1.payload)    // libère + arme le suivant
                          callback(runtime_ctx, gcHandle, &evt2) ←
                            evt2.kind = MESSAGE
                            evt2.payload = ak_bytes{ptr, len, owner}
// host peut désérialiser directement depuis evt2.payload.ptr (zero-copy recv)
ak_event_consumed(evt2.payload)    // libère + arme le suivant
                          callback(runtime_ctx, gcHandle, &evt3) ←
                            evt3.kind = STATUS  [terminal, fin du stream]
                            evt3.payload = ak_bytes{ptr, len, owner}
                            evt3.status_code = 0 (OK)
ak_event_consumed(evt3.payload)    // libère (pas de suivant, c'est le terminal)
ak_call_release(handle)
gcHandle.Free()                    // safe car terminal reçu
```

Note FFI :
- **Envoi (zero-copy)** : le buffer passé à `send_message` est lu directement par Rust (pas de
  copie). Le host DOIT garder le buffer pinned/valide jusqu'à `AK_EVENT_WRITE_DONE`. Un seul
  send en vol par call. Le terminal `AK_EVENT_STATUS` peut arriver à la place de WRITE_DONE
  (erreur/cancel — le host peut alors unpin).
- **Réception (demand via consumed)** : le payload `ak_bytes` est owned. Le host consomme
  (désérialise directement depuis le pointeur natif) puis appelle `ak_event_consumed`. Cet
  appel libère la mémoire ET arme la réception du prochain event. Un seul event non-consumed
  par call à la fois — c'est le mécanisme de backpressure.
Le terminal `AK_EVENT_STATUS` peut arriver à la place d'un message demandé (fin de stream ou erreur).

### Séquence du shutdown

```text
Host (C#)                          FFI (Rust)
─────────                          ──────────
ak_runtime_begin_shutdown(rt)      → ferme le start gate
                                     annule/draine les calls
                                     attend quiescence des tasks
                                     joint l'executor Tokio
                          callback(ctx, 0, SHUTDOWN_COMPLETE) ←  [dernier callback]
                                     thread sort du trampoline
                                     passe à AK_RELEASED
loop:
  state = ak_runtime_status(rt)
  if state == AK_RELEASED: break
  yield/spinwait
// Unload sûr
```

### Zero-copy (intégré dès la V1)

**Envoi (host → Rust)** :
- `ak_call_send_message(handle, ak_bytes_in)` — Rust lit directement le buffer host (pas de copie)
- Le host pin le buffer avant le send et le unpin après réception de `AK_EVENT_WRITE_DONE`
- Un seul send en vol par call (backpressure naturel via HTTP/2 flow control)
- Côté .NET : `GCHandle.Alloc(buffer, GCHandleType.Pinned)` ou POH (.NET 5+)

**Réception (Rust → host)** :
- `ak_event.payload` est un `ak_bytes` owned (buffer Rust reference-counted)
- Le host peut désérialiser directement depuis `payload.ptr` via `Span<byte>` ou unsafe
- Le host appelle `ak_event_consumed` quand il a fini — Rust dealloc + arme le suivant
- Pas de copie si le host consomme directement le pointeur natif

**Fragmentation mémoire** :
- Côté envoi : pas d'allocation Rust (les bytes sont dans le heap managé, pinnés)
- Côté réception : les buffers Rust sont alloués par Hyper (classes de taille similaires,
  bien gérées par jemalloc/system allocator). Si la fragmentation est mesurée en production,
  un pool de buffers pré-alloués peut être ajouté sans changement d'ABI.

---

## Couche 4 — `ArmoniK.Api.Client.RustGrpcChannel`

### Architecture interne

```text
┌──────────────────────────────────────────┐
│  NativeCallInvoker : CallInvoker         │
│    ├── NativeRuntime (SafeHandle)        │
│    ├── NativeChannel (SafeHandle)        │
│    ├── Trampoline (static delegate)      │
│    ├── HostQueue (ConcurrentQueue)       │
│    └── Dispatcher (background task)      │
└──────────────────────────────────────────┘
```

### Trampoline

```csharp
// Delegate statique rooté pour la vie du runtime.
// S'exécute sur un thread Tokio — DOIT être minimal.
private static unsafe void OnEvent(void* runtimeCtx, void* callCtx, ak_event* evt)
{
    // 1. Cast callCtx → GCHandle → CallState
    // 2. Construire un CompletionRecord (callState + kind + owned payload ref)
    // 3. Enqueue dans la HostQueue
    // 4. Signal (ManualResetEventSlim ou SemaphoreSlim)
    // 5. Retour — PAS de user code, PAS d'exception
}
```

Le trampoline ne copie PAS le payload en V1. Il stocke la référence `ak_bytes` (owned) dans le
CompletionRecord. La copie (ou la désérialisation directe) se fait dans le Dispatcher, sur un
thread managé. Le release (`ak_event_consumed`) se fait après consommation.

Plus de registry : le `call_ctx` est directement un `GCHandle` sur le `CallState` de la call,
alloué avant `ak_call_start` et libéré après réception du terminal.

### CallState (par call)

```csharp
// Alloué et GCHandle.Alloc AVANT ak_call_start.
// Le GCHandle est passé comme call_ctx. Libéré après le terminal.
class CallState
{
    TaskCompletionSource<Metadata> InitialMetadataTcs;  // RunContinuationsAsynchronously
    Channel<OwnedMessage> MessageChannel;               // demand-driven via ak_call_request_next_message
    TaskCompletionSource<GrpcStatus> StatusTcs;         // terminal
    CancellationTokenRegistration CancelRegistration;
    GCHandle SelfHandle;                                // le GCHandle passé comme call_ctx
}
```

### Dispatcher

Le dispatcher est une task en background qui draine la HostQueue :

```text
while (queue.TryDequeue(out record) || await signal):
    state = registry.Lookup(record.Token)
    switch record.Kind:
        INITIAL_METADATA → state.InitialMetadataTcs.SetResult(decode(record.Payload))
                           ak_event_consumed(record.Payload)
        MESSAGE          → state.MessageChannel.Write(record.Payload)  // consumed après désérialisation
        STATUS           → state.StatusTcs.SetResult(decode(record.Payload))
                           ak_event_consumed(record.Payload)
                           registry.Remove(record.Token)
```

### NativeCallInvoker — mapping CallInvoker

Les 5 méthodes de `CallInvoker` se traduisent ainsi :

| Méthode CallInvoker | Implémentation |
|---------------------|----------------|
| `BlockingUnaryCall` | start + send + end_send + await status (bloque le thread) |
| `AsyncUnaryCall` | start + send + end_send + return Task wrapper |
| `AsyncClientStreamingCall` | start + expose write stream + return Task<response> |
| `AsyncServerStreamingCall` | start + send + end_send + expose read stream |
| `AsyncDuplexStreamingCall` | start + expose write stream + expose read stream |

Chaque appel :
1. Sérialise la request en bytes (protobuf, fait par le stub)
2. Alloue un `CallState`, fait `GCHandle.Alloc` dessus
3. Appelle `ak_call_start` avec le GCHandle comme `call_ctx`
4. Retourne l'objet adapté (AsyncUnaryCall, etc.) qui wraps les TCS du CallState

### Configuration — chaîne de génération

La chaîne complète est :

```text
Types Rust (TransportConfig, GrpcChannelConfig)
    │ derive(schemars::JsonSchema) sur les types *Source (sérialisables)
    ▼
channel_config.schema.json  ← commité, source de vérité
    │ outil de génération (NJsonSchema, ou custom)
    ▼
RustChannelOptions.g.cs     ← généré, types C# pour configurer le channel
    │ .ToJson()
    ▼
JSON UTF-8 passé à ak_channel_create
    │ serde::Deserialize côté Rust
    ▼
GrpcChannelConfig (types Rust, avec *Source)
    │ .load() / .resolve()
    ▼
Matériel effectif (Identity, CA certs, route proxy, etc.)
```

Les types `*Source` (IdentitySource, CaSource, ProxySource) sont sérialisables (serde +
schemars). Le matériel chargé (Identity, CertificateDer) ne l'est pas. Le schéma JSON est
généré depuis les types source, ce qui garantit la cohérence entre les options Rust et les
options C# sans maintenance manuelle.

---

## Couche 5 — `ArmoniK.Api.Client`

### Point d'intégration

```csharp
// Le client existant accepte un CallInvoker injectable :
public class SessionsClient
{
    public SessionsClient(CallInvoker callInvoker) { ... }
}

// Usage avec le channel natif :
var options = new RustChannelOptions { Endpoint = "https://armonik:5001" };
using var invoker = new NativeCallInvoker(options);
var client = new Sessions.SessionsClient(invoker);
```

### Mapping d'options existantes

Les options actuelles du client (`GrpcChannel` dans ArmoniK.Api.Common.Options) doivent pouvoir
produire un `RustChannelOptions`. Le mapping est explicite et testé :

| Option existante | Champ RustChannelOptions |
|-----------------|--------------------------|
| `Address` | `Endpoint` |
| `CaCert` | `Tls.CaCertPath` |
| `ClientCert` / `ClientKey` | `Tls.ClientIdentity` (PEM) |
| `ClientP12` | `Tls.ClientIdentity` (PKCS12) |
| `AllowUnsafeConnection` | `Tls.CaSource = Insecure` |
| `OverrideTargetName` | `Tls.OverrideTargetName` |
| `Proxy` | `Proxy.Source` |
| `ProxyUsername` / `ProxyPassword` | `Proxy.Credentials` |
| `RequestTimeout` | `DefaultDeadline` |
| `MaxAttempts` | `Retry.MaxAttempts` |
| `InitialBackOff` etc. | `Retry.*` |

---

## Modèle formel TLA+

### Structure et emplacement

Les fichiers TLA+ vivent dans `spec/armonik_grpc_ffi/tla/`. Le modèle est structuré en trois
niveaux de refinement :

```text
Niveau 0 — Spec abstraite (ce que l'utilisateur observe)
    AbstractGrpc.tla

Niveau 1 — Spec FFI (ce qui se passe à la frontière C)
    FfiGrpc.tla  refines  AbstractGrpc

Niveau 2 — Spec binding .NET (ce qui se passe côté managé)
    DotNetBinding.tla  refines  FfiGrpc
```

### Niveau 0 — AbstractGrpc

Variables d'état :
- `runtime_state` : NOT_INIT | RUNNING | STOPPING | DRAINING | RELEASED | FAILED_UNQUIESCED
- `channels` : ensemble de channels (open | closed)
- `calls` : ensemble de calls avec leur état
- `send_closed` : booléen par call (end_send appelé)
- Par call, 4 séquences de messages :
  - `submitted` : messages soumis par le client à la lib (via send_message)
  - `sent` : messages effectivement envoyés sur le réseau (HTTP/2)
  - `received` : messages reçus du réseau (HTTP/2)
  - `delivered` : messages délivrés au client par la lib (via callback/next_message)
- `events_delivered` : séquence ordonnée d'events par call (INITIAL_METADATA, MESSAGE*, STATUS)

#### Invariants de safety (à prouver par TLAPS)

**Séquencement des events par call :**
- **MetadataFirst** : ∀ call : le premier event dans `events_delivered` est INITIAL_METADATA (si la call reçoit des events)
- **StatusLast** : ∀ call : STATUS est le dernier event (terminal), il arrive exactement une fois
- **NoEventAfterStatus** : ∀ call : aucun event après STATUS

**Intégrité des messages (invariants de préfixe, liveness d'égalité) :**
- **SubmittedPrefixOfSent** : ∀ call, à tout instant : `sent` est un préfixe de `submitted`
- **ReceivedPrefixOfDelivered** : ∀ call, à tout instant : `delivered` est un préfixe de `received`
- **SentEqualsSubmitted** : ∀ call terminée avec succès : `sent` = `submitted` (liveness)
- **DeliveredEqualsReceived** : ∀ call terminée avec succès : `delivered` = `received` (liveness)
- **SubmitProgress** : ∀ message m ∈ `submitted` : ◇ (m ∈ `sent` ∨ call terminée/annulée)
- **DeliveryProgress** : ∀ message m ∈ `received` : ◇ (m ∈ `delivered` ∨ call terminée/annulée)

**Unicité et terminaison :**
- **UniqueTerminal** : ∀ call : |{evt ∈ events | evt.kind = STATUS}| ≤ 1
- **SendAfterEndSend** : ∀ call : send_closed[call] ⇒ ¬∃ send_message(call) futur

**Runtime lifecycle — transitions monotones :**
- **SingleRuntime** : au plus un runtime avec state ∈ {RUNNING, STOPPING, DRAINING} à tout instant
- **MonotoneRuntime** : les transitions suivent NOT_INIT → RUNNING → STOPPING → DRAINING → RELEASED (pas de retour)
- **ReleasedTerminal** : runtime_state = RELEASED ⇒ aucune transition future
- **FailedTerminal** : runtime_state = FAILED_UNQUIESCED ⇒ aucune transition future

**Ownership — tout appartient à un runtime :**
- **ChannelOwnership** : ∀ channel : ∃! runtime tel que channel ∈ runtime.channels
- **CallOwnership** : ∀ call : ∃! channel tel que call ∈ channel.calls (et donc ∃! runtime)
- **NoOrphan** : aucun channel ni call n'existe en dehors d'un runtime

**Lien channels ↔ runtime :**
- **CreateRequiresRunning** : un channel ne peut être créé que si runtime_state = RUNNING
- **StoppingClosesChannels** : runtime_state = STOPPING ⇒ ∀ channel : channel.state ∈ {closing, closed}
- **ReleasedNoChannels** : runtime_state = RELEASED ⇒ ∀ channel : channel.state = closed

**Lien calls ↔ runtime :**
- **StartRequiresOpenChannel** : une call ne peut être startée que si channel.state = open (et donc runtime = RUNNING)
- **StoppingTerminatesCalls** : runtime_state ∈ {STOPPING, DRAINING} ⇒ ∀ call active : ◇ STATUS délivré
- **ReleasedNoCalls** : runtime_state = RELEASED ⇒ aucune call active, aucun callback en vol

#### Liveness (conditionnelle aux fairness)

- **EventualTerminal** : 
  ∀ call started : ◇ STATUS délivré
  (sous : scheduler fairness, réseau progresse, client et serveur
  produisent chacun un nombre fini de messages)
- **EventualShutdown** : 
  runtime_state = STOPPING ⇒ ◇ runtime_state = RELEASED
  (sous : consumer draine, callbacks retournent, peer répond ou timeout)
- **EventualMessage** : 
  request_next_message appelé ∧ serveur a encore des messages ⇒ ◇ MESSAGE délivré
  (sous : réseau progresse)

### Niveau 1 — FfiGrpc

Variables ajoutées :
- `handles` : registry (id → resource, generation)
- `callbacks_in_flight` : compteur par call
- `start_gate` : open | closed
- Par call, 2 états intermédiaires de message ajoutés aux séquences du niveau 0 :
  - `at_ffi_boundary_send` : messages acceptés par la FFI mais pas encore passés au channel Rust
    (entre `submitted` et `sent`)
  - `at_ffi_boundary_recv` : messages reçus du channel Rust mais pas encore délivrés au host
    via callback (entre `received` et `delivered`)

Invariants additionnels :
- **HandleValidity** : tout downcall accepté utilise un handle valide dans la registry
- **BorrowedLifetime** : buffer send non lu par Rust après émission de WRITE_DONE ou STATUS
- **CallbackSerialization** : callbacks_in_flight[call] ≤ 1
- **SingleSendInFlight** : ∀ call : au plus un send en attente de WRITE_DONE à la fois
- **SingleRecvInFlight** : ∀ call : au plus un event non-consumed à la fois (le runtime ne
    délivre pas le suivant tant que le précédent n'est pas consumed)
- **GateClosed** : start_gate = closed ⇒ ¬∃ nouveau child créé
- **ReleasedImpliesQuiescent** : runtime_state = RELEASED ⇒
    callbacks_in_flight = 0 ∧ handles = ∅ ∧ tasks = ∅
- **FfiBoundaryOrder** : ∀ call : `at_ffi_boundary_send` est un suffixe de `submitted` et un
    préfixe de `sent` ; `at_ffi_boundary_recv` est un suffixe de `received` et un préfixe de
    `delivered`

#### Stratégie de preuve des liveness (décomposition des fairness)

Les liveness du niveau 0 (EventualTerminal, EventualShutdown, SubmitProgress, DeliveryProgress)
ne sont **pas reprouvées** au niveau 1. Au lieu de cela, on prouve que les fairness locales du
niveau 1 impliquent les fairness du niveau 0 :

```text
Fairness niveau 0             Décomposée en fairness locales niveau 1
─────────────────             ─────────────────────────────────────────
"réseau progresse"         =  "Tokio scheduler fair" ∧ "connexion HTTP/2 progresse"
"client produit fini"      =  "host appelle send_message un nombre fini de fois"
"serveur produit fini"     =  "peer envoie END_STREAM" ∧ "Tokio lit les frames"
"scheduler fairness"       =  "Tokio repoll les tasks réveillées"
                              ∧ "callback trampoline retourne"
                              ∧ "host dispatcher route les events"
```

Chaque fairness locale est justifiable par une couche :
- "Tokio repoll" → garantie Tokio (OS accorde du CPU)
- "callback retourne" → raffiné au niveau 2 (.NET binding : trampoline borné, pas de user code)
- "host dispatcher route" → thread managé dédié, signalé

Le refinement prouve :
1. Les invariants de safety du niveau 1 ⇒ les invariants de safety du niveau 0
2. Les fairness locales du niveau 1 ⇒ les fairness du niveau 0
3. Donc les liveness du niveau 0 sont héritées par composition, pas reprouvées

Refinement mapping vers AbstractGrpc :
- Un `ak_call_start` accepté ↔ une call started
- `AK_EVENT_MESSAGE` délivré ↔ message_received ajouté
- `AK_EVENT_STATUS` délivré ↔ status fixé (terminal)
- `ak_call_end_send` ↔ transition vers half_closed
- `ak_channel_release` ↔ channel closed

### Niveau 2 — DotNetBinding

Variables ajoutées :
- `call_states` : GCHandle → CallState (alloué avant start, libéré après terminal)
- `host_queue` : séquence de CompletionRecords
- `tcs_state` : par call, état de chaque TaskCompletionSource
- `gc_roots` : ensemble de GCHandle vivants
- `dispose_state` : active | disposing | disposed

Invariants additionnels :
- **RootSurvivesCallbacks** : ∀ call active : gc_root(runtime_ctx) ∈ gc_roots
- **TokenPublishedBeforeStart** : ∀ call : GCHandle(call_ctx) alloué avant ak_call_start
- **ContinuationsAsync** : TCS complétée ⇒ continuation non-inline
- **DisposeAwaitsReleased** : dispose complété ⇒ runtime_state = RELEASED

Refinement mapping vers FfiGrpc :
- `GCHandle.Alloc(callState)` avant start ↔ publication avant start
- `OnEvent` trampoline ↔ callback reception
- `Dispatcher` route ↔ event consommé
- `ak_event_consumed` appelé ↔ payload released + demand signal
- `DisposeAsync` complété ↔ RELEASED observé

### Preuve TLAPS

La stratégie de preuve est :
1. Prouver les invariants de safety de chaque niveau indépendamment (induction)
2. Les liveness du niveau O sont prouvées avec les fairness de ce niveau
3. Prouver le refinement entre niveaux (simulation)
4. Si possible, les liveness du refinement sont prouvées en prouvant que les fairness + safety du niveau courant implémentent les fairness du niveau supérieur.

Les fichiers de preuve seront dans `spec/armonik_grpc_ffi/tla/proofs/`.

---

## Décisions ouvertes (à résoudre pendant l'implémentation)

| Question | Options | Impact |
|----------|---------|--------|
| Crate pour X509Store Windows | API natives directes / crate `schannel` / crate `windows` | Couche 1 |
| Format exact des handles (pointeur opaque vs index + generation) | Performance vs safety | Couche 3 |
| Taille par défaut du buffer de rejeu | 0 (pas de retry streaming) vs 4KB vs 64KB | Couche 2 config |
| Mécanisme de signal host queue | `ManualResetEventSlim` vs `SemaphoreSlim` vs custom | Couche 4 perf |
| Source generator vs T4 pour options C# | Outillage build | Couche 4 |
| Gestion du pool de connexions (idle eviction) | Timer interne vs lazy check | Couche 2 |

---

## Références

- [gRPC over HTTP/2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
- [gRPC Retry Design](https://github.com/grpc/proposal/blob/master/A6-client-retries.md)
- [tower::Service](https://docs.rs/tower/latest/tower/trait.Service.html)
- [Grpc.Core.CallInvoker](https://grpc.github.io/grpc/csharp-dotnet/api/Grpc.Core.CallInvoker.html)
- [TLA+ Proof System](https://tla.msr-inria.inria.fr/tlaps/content/Home.html)
