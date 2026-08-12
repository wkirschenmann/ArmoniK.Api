# SPEC — Client .NET ArmoniK sur channel gRPC natif Rust

## Objectifs

1. Remplacer le channel gRPC managé (`Grpc.Net.Client` / `WinHttpHandler`) par un channel gRPC
   natif Rust commun, exposé à .NET via une FFI C mince.
2. **Unifier le comportement** du client ArmoniK quel que soit le runtime .NET (Framework 4.7.2,
   4.8, .NET 6+, .NET 8+). Le transport natif élimine les divergences de comportement entre
   `WinHttpHandler`, `SocketsHttpHandler`, `GrpcWebHandler` et les fallbacks conditionnels.
3. Rendre effectives les options aujourd'hui inopérantes (`RequestTimeout`, `OverrideTargetName`).
4. Conserver l'API publique du client : même surface, mêmes stubs générés. Seul le `CallInvoker`
   sous-jacent change.

---

## Architecture en couches

```text
┌─────────────────────────────────────────────────────────┐
│  5. ArmoniK.Api.Client                                  │  C# — netstandard2.0
│     Stubs gRPC générés, services ArmoniK                │
│     CallInvoker injectable (managé OU natif)            │
├─────────────────────────────────────────────────────────┤
│  4. ArmoniK.Api.Client.RustGrpcChannel                  │  C# — netstandard2.0
│     CallInvoker natif, runtime de binding,              │  (multitarget si justifié)
│     trampoline, host queue, SafeHandles, shutdown       │
├─────────────────────────────────────────────────────────┤
│  3. armonik-grpc-channel-ffi                            │  Rust → DLL/SO C
│     ABI C versionnée, runtime Tokio possédé,            │
│     handles, callbacks, registries, shutdown            │
├─────────────────────────────────────────────────────────┤
│  2. armonik-grpc-channel                                │  Rust — crate lib
│     Channel gRPC fonctionnel : framing, metadata,       │
│     status, retry, deadline, flow control, 4 cardinal.  │
├─────────────────────────────────────────────────────────┤
│  1. armonik-transport                                   │  Rust — crate lib
│     Connecteur réseau : endpoint, TCP, TLS/mTLS,        │
│     proxy, connect timeout, DNS                         │
└─────────────────────────────────────────────────────────┘
```

### Multitargeting .NET

Le package `ArmoniK.Api.Client.RustGrpcChannel` cible `netstandard2.0` comme base. Un
multitargeting (ex: `net6.0`, `net8.0`) est autorisé s'il apporte un bénéfice concret (ex:
`LibraryImport` pour réduire l'overhead P/Invoke, `NativeMemory`, span-based APIs). Le
comportement fonctionnel reste identique quelle que soit la TFM — c'est précisément l'objectif
d'unification.

---

## Couche 1 — `armonik-transport` (compléter)

### État actuel

Le crate existe et fournit :
- Configuration et résolution endpoint/origin
- TCP et options socket
- TLS, mTLS, CA explicite et racines natives
- Server-name override (déclaré mais pas effectif)
- Proxy HTTP CONNECT
- Connect timeout
- Un `Connector` consommable par Hyper/Tonic

### Fonctionnalités manquantes

- **Contrat public stable du `Connector`** — La frontière entre ce que `armonik-transport` expose
  et ce que `armonik-grpc-channel` consomme n'est pas formalisée. C'est le prérequis à tout le
  reste : le trait/type public du connecteur, ses garanties, son contrat d'erreur.

- **Extraction des options qui ne lui appartiennent pas** — `RetryConfig`, `Http2Config`, request
  timeout et options de pool sont actuellement déclarés ici mais appliqués au-dessus. Ils doivent
  migrer vers `armonik-grpc-channel`. La configuration résiduelle de `armonik-transport` est :
  endpoint, TLS/mTLS, TCP, proxy, connect timeout.

- **`OverrideTargetName` effectif** — L'option existe mais ne modifie pas le nom vérifié par
  rustls. Elle doit agir sur le `ServerName` du handshake TLS.

- **`ProxySource::WindowsSystem`** — Lecture du proxy système Windows via WinHTTP (resolver
  seulement, pas moteur réseau). Résolution async avec timeout et cancellation ; ne doit jamais
  bloquer sous le loader lock ou dans un callback FFI.

- **Erreurs réseau structurées** — Les messages d'erreur ne doivent contenir ni secrets (paths de
  clé privée, passwords) ni informations de debug internes en mode release. Codes d'erreur
  discriminables programmatiquement.

---

## Couche 2 — `armonik-grpc-channel` (créer)

### Responsabilité

Moteur gRPC fonctionnel Rust pur. Aucune notion de C ABI, callback ou handle foreign. Consommable
directement par un client Rust via adapter Tonic, et par la couche FFI.

### Modèle gRPC — mapping sur HTTP/2

Un appel gRPC est une requête HTTP/2 POST :

```text
Requête HTTP/2 :
  :method = POST
  :path = /{service}/{method}
  :scheme = http | https
  content-type = application/grpc
  te = trailers
  grpc-timeout = {deadline}         ← optionnel, transmis au serveur
  {metadata clé: valeur}            ← metadata utilisateur = headers HTTP additionnels
  {metadata clé-bin: base64(bytes)} ← metadata binaire

  [body] = suite de messages gRPC framed :
    [1 byte compressed flag][4 bytes big-endian length][message bytes]

Réponse HTTP/2 :
  :status = 200
  content-type = application/grpc
  {initial metadata}                ← headers HTTP de la réponse

  [body] = suite de messages gRPC framed (même format)

  Trailers HTTP/2 (trailing headers) :
    grpc-status = {code}
    grpc-message = {message}        ← optionnel
    {trailing metadata}             ← metadata utilisateur
```

**Metadata** = headers HTTP de la requête/réponse. On les passe dans les headers HTTP/2 de la
requête (pour les request metadata) et on les lit dans les headers HTTP/2 de la réponse (initial
metadata) et dans les trailers HTTP/2 (trailing metadata contenant aussi `grpc-status`).

**Half-close** (`end_send`) = le client signale qu'il n'enverra plus de messages sur cette call.
Concrètement c'est un `END_STREAM` sur le request body HTTP/2. Pour un unary ou client-streaming,
c'est ce qui déclenche le traitement côté serveur. Le terme `half_close` vient du protocole gRPC ;
dans notre API on l'appellera `end_send` pour la clarté.

### Options de connexion

Le channel supporte :
- **Connexion lazy** (par défaut) — la première connexion physique s'établit au premier appel
- **Connexion eager** (option de configuration) — une connexion HTTP/2 est établie dès la création
  du channel, sans attendre le premier appel

### Fonctionnalités V1

- Client/pool Hyper HTTP/2, connexion lazy ou eager (configurable)
- Framing/déframing gRPC (length-prefixed messages)
- Les 4 cardinalités (unary, client streaming, server streaming, bidi streaming)
- Request metadata → headers HTTP/2 de la requête
- Initial metadata ← headers HTTP/2 de la réponse
- Trailing metadata ← trailers HTTP/2 (incluant `grpc-status` et `grpc-message`)
- Status gRPC extrait des trailers
- Flow control (demand-driven reads)
- `end_send` (half-close : END_STREAM sur le request body)
- Cancellation locale et propagation RST_STREAM
- Deadline locale + header `grpc-timeout` transmis au serveur
- Retry/backoff (5 attempts, initial 1s, max 5s, multiplier 1.5, codes UNAVAILABLE/ABORTED/UNKNOWN)
- Notion de call committed (pas de retry après envoi de données en streaming)
- Pool HTTP/2 avec keepalive et idle timeout
- Options HTTP/2 (max concurrent streams, initial window size, etc.)
- Pas de runtime Tokio possédé — utilise un `Executor` trait fourni par le caller

### Hors V1

- Hedging, load balancing, compression, interceptors, channelz, connectivity state

### Executor trait

```rust
/// Le channel spawn des tasks via cet executor.
/// En Rust normal : adapter Tokio direct.
/// En FFI : adapter qui enregistre dans le task group du runtime FFI.
pub trait Executor: Send + Sync + 'static {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) -> TaskHandle;
}
```

---

## Couche 3 — `armonik-grpc-channel-ffi` (créer)

### Responsabilité

Projection C de la couche 2. Possède l'isolation foreign : runtime Tokio, handles, callbacks,
registries, task tracking, shutdown et unload sûr.

### Surface ABI (esquisse — sera finalisée dans DESIGN.MD)

```c
// === Lifecycle runtime ===
ak_status ak_runtime_create(const ak_runtime_config *config,
                            ak_callback callback,
                            void *runtime_ctx,
                            ak_runtime_handle *out_runtime);
ak_runtime_state ak_runtime_status(ak_runtime_handle runtime);
ak_status ak_runtime_begin_shutdown(ak_runtime_handle runtime);

// === Channel ===
ak_status ak_channel_create(ak_runtime_handle runtime,
                            ak_bytes_in config_json,
                            ak_channel_handle *out_channel);
void ak_channel_release(ak_channel_handle channel);
// Release est synchrone. Les calls en cours sur ce channel reçoivent un
// terminal en erreur (CANCELLED). Le channel conserve ses ressources internes
// jusqu'à quiescence de ses calls, mais le handle n'est plus utilisable
// pour démarrer de nouvelles calls.

// === Call gRPC ===
ak_status ak_call_start(ak_channel_handle channel,
                        const ak_call_start_options *options,
                        ak_operation_token token,
                        ak_call_handle *out_call);
ak_status ak_call_send_message(ak_call_handle call, ak_bytes_in message);
ak_status ak_call_end_send(ak_call_handle call);
ak_status ak_call_request_messages(ak_call_handle call, uint32_t n);
ak_status ak_call_cancel(ak_call_handle call);
void ak_call_release(ak_call_handle call);

// === Utilitaires ===
int ak_abi_version(void);
void ak_bytes_release(ak_bytes bytes);
```

### Callback (trampoline contract)

```c
typedef ak_callback_result (*ak_callback)(
    void *runtime_ctx,
    ak_operation_token token,
    const ak_event *event);  // payload borrowed pendant l'appel
```

Events délivrés par call via le callback :

| Event | Payload | Quand |
|-------|---------|-------|
| `AK_EVENT_INITIAL_METADATA` | Metadata (headers HTTP/2 réponse) | Dès réception des response headers |
| `AK_EVENT_MESSAGE` | Bytes du message gRPC (déjà déframé) | Pour chaque message demandé via `request_messages` |
| `AK_EVENT_STATUS` | Status gRPC + trailing metadata | **Terminal** — dernier event de la call |

Events runtime :

| Event | Payload | Quand |
|-------|---------|-------|
| `AK_EVENT_SHUTDOWN_COMPLETE` | Status du shutdown | Dernier callback du runtime |

`AK_EVENT_STATUS` est le terminal de la call. Il contient le `grpc-status`, le `grpc-message` et
les trailing metadata (extraits des trailers HTTP/2). Toute information de la call — initial
metadata, messages, status — est délivrée via le callback. Les messages arrivent seulement en
réponse à `ak_call_request_messages` (demand-driven).

### Sémantique de `ak_channel_release`

- Synchrone et non bloquant
- Le handle n'est plus valide pour `ak_call_start` après l'appel
- Les calls en cours sont annulées (reçoivent un `AK_EVENT_STATUS` avec code CANCELLED)
- Les ressources internes du channel sont libérées après quiescence des calls
- Le runtime conserve la trace du channel jusqu'à sa quiescence (pas de fuite)

### Propriétés de safety

- Handles opaques, validés dans une registry par runtime (generation-tagged)
- Inputs `ptr+len` borrowed pendant le downcall, copiés par Rust
- Callbacks sérialisés par call, concurrents entre calls
- Un seul terminal (`AK_EVENT_STATUS`) par call acceptée
- Aucun callback après le terminal d'une call
- Start gate fermé → aucun nouveau child
- `AK_RELEASED` certifie la quiescence (unload autorisé)

### Lifecycle runtime

```text
AK_NOT_INITIALIZED → AK_RUNNING → AK_STOPPING → AK_DRAINING → AK_RELEASED
                                                      ↓
                                          AK_FAILED_UNQUIESCED
```

### Configuration

La configuration du channel est passée en JSON UTF-8. Le schéma JSON est la source de vérité unique
pour les options (partagé entre toutes les couches). Le schéma vit dans le crate FFI et est généré
depuis les types Rust de `armonik-grpc-channel` + `armonik-transport`. Il est commité dans le repo
pour que les bindings puissent générer leur code.

---

## Couche 4 — `ArmoniK.Api.Client.RustGrpcChannel` (créer)

### Responsabilité

Package NuGet C# qui implémente `Grpc.Core.CallInvoker` au-dessus de la FFI. Contient le runtime
de binding .NET.

### Composants internes

| Composant | Rôle |
|-----------|------|
| `NativeRuntime` | Chargement DLL, version check, `SafeHandle` sur `ak_runtime` |
| `NativeChannel` | `SafeHandle` sur `ak_channel`, config JSON |
| `NativeCallInvoker` | Implémente `CallInvoker` (5 méthodes) |
| `Trampoline` | Delegate static rooted, copie payload, enfile `CompletionRecord` |
| `OperationRegistry` | Token → `CallState`, publication avant `ak_call_start` |
| `HostQueue` | `ConcurrentQueue<CompletionRecord>` + signal |
| `Dispatcher` | Thread/task dédié, route les completions vers les TCS |
| `ShutdownCoordinator` | `begin_shutdown` → poll `AK_RELEASED` → dispose |

### Configuration

La classe d'options C# (`RustChannelOptions` ou équivalent) est **générée à partir du schéma JSON**
de configuration du FFI. Cela garantit la cohérence entre les options C# et les options Rust sans
maintenance manuelle.

### Contraintes

- `netstandard2.0` comme TFM de base (compatible Framework 4.7.2+)
- Multitarget `net6.0`/`net8.0` autorisé si bénéfice mesurable (ex: `LibraryImport`)
- `DllImport` avec `CallingConvention.Cdecl` sur netstandard2.0
- `GCHandle` pour `runtime_ctx`, publié avant tout `ak_call_start`
- `TaskCompletionSource` avec `RunContinuationsAsynchronously`
- Cancellation via `CancellationToken` → `ak_call_cancel`
- Aucun user code dans le trampoline

### Packaging natif

Binaire natif par RID inclus dans le package NuGet :

| RID | Plateforme |
|-----|-----------|
| `win-x64` | Windows 64-bit |
| `win-x86` | Windows 32-bit |
| `linux-x64` | Linux 64-bit |
| `linux-x86` | Linux 32-bit |
| `linux-arm64` | Linux ARM 64-bit |

Structure : `runtimes/{rid}/native/armonik_grpc_channel_ffi.{dll|so}`

---

## Couche 5 — `ArmoniK.Api.Client` (compléter)

### État actuel

- Cible `netstandard2.0`
- Utilise `Grpc.Net.Client` ou `Grpc.Core` selon la plateforme/runtime
- `GrpcChannelProvider` fournit un `ChannelBase`
- Les stubs générés appellent un `CallInvoker`
- Comportement divergent entre .NET Framework (WinHttpHandler/GrpcWeb) et .NET moderne
  (SocketsHttpHandler)
- `RequestTimeout` ne pose aucune deadline (warning seulement)
- `OverrideTargetName` n'a aucun effet (warning seulement)

### Fonctionnalités manquantes

- **CallInvoker injectable** — Pouvoir fournir un `CallInvoker` externe (le `RustCallInvoker`) sans
  passer par `ChannelBase`. C'est le point d'intégration entre les couches 4 et 5.

- **Provider natif** — Un `RustGrpcChannelProvider` (ou adaptation du `GrpcChannelProvider`
  existant) qui construit le channel natif depuis les options existantes.

- **Mapping d'options** — Les options de configuration existantes (`GrpcChannel`, `TlsOptions`,
  etc.) doivent pouvoir produire le JSON attendu par la FFI. La correspondance doit être explicite
  et testée.

- **Deadline effective** — `RequestTimeout` doit produire une deadline réelle transmise au serveur
  via `grpc-timeout` et appliquée localement.

- **OverrideTargetName effectif** — L'option doit modifier le server name vérifié par TLS.

- **Shutdown/dispose propre** — `IAsyncDisposable` qui déclenche le shutdown du runtime natif et
  attend `AK_RELEASED` avant de permettre l'unload.

- **Fallback managé opt-in** — Le channel natif est le défaut quand la DLL native est disponible.
  Si elle est absente ou si l'utilisateur le demande, retomber sur le comportement managé actuel.
  Cela ne doit pas casser les déploiements existants.

- **Comportement unifié** — Quel que soit le runtime .NET (4.7.2, 4.8, 6, 8), les options
  produisent le même effet réseau, car le transport est identique (Rust natif).

---

## Modèle formel TLA+

### Emplacement

Les spécifications TLA+ vivent dans :

```text
spec/armonik_grpc_ffi/tla/
├── AbstractGrpc.tla          channel, calls, terminal, cancel, retry (spec fonctionnelle)
├── CallLifecycle.tla         start, send, end_send, recv, terminal, quiescence
├── GrpcFlowControl.tla       message demand, send admission, bornes
├── Retry.tla                 attempts, commitment, backoff, deadline
├── FfiRuntime.tla            gates, children, tasks, callbacks, shutdown
├── GenerationalIds.tla       allocation, validation, tombstones
├── CallbackProtocol.tla      publication, trampoline, résultat, barrier
├── MemoryOwnership.tla       borrowed, copied, retained, released
├── FailureContainment.tla    fault latch, quarantine, quiescence
├── FfiGrpc.tla               composition concrète FFI + gRPC
├── FfiRefinement.tla         mapping FfiGrpc → AbstractGrpc
├── DotNetBinding.tla         roots, queue, TCS, cancellation, dispose
└── DotNetRefinement.tla      mapping DotNetBinding → FfiGrpc
```

### Rôle

Les modèles TLA+ vérifient les propriétés de safety (invariants) et de liveness conditionnelle
(progress sous fairness assumptions). Ils ne prouvent pas la performance, la conformité du code, ni
la terminaison de code foreign arbitraire.

La spec TLA+ sera détaillée dans le DESIGN.MD. Ce qui est fixé ici :
- La spec abstraite (`AbstractGrpc.tla`) est la cible du premier refinement
- Le FFI et le binding .NET sont des niveaux de refinement successifs
- Les safety properties ne dépendent d'aucune fairness assumption
- Les liveness sont conditionnelles aux fairness déclarées (scheduler, callback, consumer, peer)

---

## Périmètre V1

### Inclus

- Endpoint HTTP et HTTPS
- HTTP/2 gRPC natif (pas de gRPC-Web)
- Connexion lazy (défaut) ou eager (option)
- 4 cardinalités gRPC
- Request metadata (headers HTTP/2 de la requête)
- Initial metadata (headers HTTP/2 de la réponse)
- Trailing metadata + status gRPC (trailers HTTP/2)
- Cancellation
- Deadline réellement appliquée et transmise (`grpc-timeout`)
- Retry (5 attempts, 1s→5s, ×1.5, UNAVAILABLE/ABORTED/UNKNOWN)
- TLS avec racines système
- CA PEM explicite
- Connexion non vérifiée (opt-in explicite)
- mTLS (P12/PFX ou PEM+key)
- OverrideTargetName effectif
- Proxy : disabled, explicit, environment, Windows system
- Auth proxy Basic
- Connect timeout
- Keepalive TCP, idle timeout
- Shutdown déterministe et unload sûr
- Comportement identique sur .NET Framework 4.7.2/4.8 et .NET 6/8+

### Hors V1

- Télémétrie OpenTelemetry
- Hedging
- Load balancing générique
- Compression configurable
- Interceptors génériques
- gRPC-Web
- Bindings Java/Python/C++

---

## Décisions de design

1. **Callback MsQuic-style vers trampoline officiel** — pas de completion queue exposée à l'ABI
2. **Un seul channel gRPC natif** — pas de duplication de retry/framing dans le binding
3. **`CallInvoker`** comme point d'intégration .NET (pas `HttpMessageHandler`) — évite la
   dépendance à `Grpc.Net.Client` pour le transport
4. **Consumer token publié avant start** — ferme la race fast-callback
5. **Runtime Tokio possédé par le FFI** — pas de singleton process-wide
6. **Handles generation-tagged** — élimine l'ABA problem
7. **Configuration immuable après création** — snapshot au start de chaque call
8. **`AK_RELEASED` comme unique condition d'unload** — safety avant liveness
9. **`ak_channel_release` synchrone avec annulation des calls** — simplifie le lifecycle host
10. **Schéma JSON source de vérité** — options C# générées depuis le schéma, pas l'inverse
11. **Connexion eager optionnelle** — configurable, lazy par défaut
12. **`end_send` plutôt que `half_close`** — clarté du nommage dans l'API

---

## Critères de validation

### Core Rust (couches 1+2)

- Configuration sans ouverture de socket (mode lazy)
- Connexion eager si configurée
- 4 cardinalités contre serveur réel
- Metadata request/response/trailing, trailers-only, status non-OK
- Gros messages au-delà de la fenêtre HTTP/2
- Slow consumer et flow control borné (demand-driven)
- Cancellation avant/après headers
- Timeout avant/après headers
- end_send et races terminales
- Retries et commitment par cardinalité
- Calls concurrentes sur un channel
- Fermeture du channel pendant calls actives
- Proxy (explicit, env, Windows system)
- TLS, CA, mTLS, target override

### FFI (couche 3)

- Callback rapide avant retour du start (consumer token publié avant)
- Callbacks sérialisés par call, concurrents entre calls
- Tokens stale/wrong-owner/wrong-generation rejetés
- Terminal unique (`AK_EVENT_STATUS`), pas d'event tardif
- Release concurrent avec callback/cancel/terminal
- Inputs copiés avant retour
- Panic containment
- Start refusé après fermeture du gate
- Task group vide après shutdown
- Callback barrier sans callback tardif
- Poll `AK_RELEASED` certifie la quiescence
- `ak_channel_release` annule les calls en cours proprement

### .NET (couches 4+5)

- Tests net472 et net48
- Tests net6.0 et net8.0 (comportement identique)
- Stubs générés utilisant le `CallInvoker` natif
- Unary bloquant et async
- Client, server et bidirectional streaming
- CancellationToken et deadline effective
- Continuations non-inline (`RunContinuationsAsynchronously`)
- Rooting des delegates sous GC forcé
- Dispose pendant call active
- Cycles répétés create/shutdown/status → `AK_RELEASED`
- Unload uniquement après quiescence démontrée
- Options existantes préservées et effectives
- Fallback managé fonctionne si DLL native absente

---

## Risques et mitigations

| Risque | Impact | Mitigation |
|--------|--------|------------|
| Incompatibilité .NET Framework 4.7.2 | Bloquant | Tests CI net472 dès les premiers développements |
| Performance du trampoline + host queue | Latence | Benchmark vs baseline managée tôt |
| Complexité du shutdown sous GC/finalizers | Corruption mémoire | Modèle TLA+ du binding, pas de finalizer qui force l'unload |
| Cross-compilation multi-RID | Packaging | CI matrix dès le début |
| Options de proxy Windows (PAC/WPAD) | Timeout/hang | WinHTTP resolver async avec timeout |
| Divergence schéma JSON / options C# | Drift | Génération C# depuis le schéma, test de round-trip |

---

## Références

- `packages/rust/armonik-transport-ffi/TOWARDS_ARMONIK_GRPC.MD` — Synthèse de transition
- `packages/rust/armonik-transport-ffi/DESIGN.md` — Design de l'isolation FFI asynchrone
- `packages/rust/armonik-transport-ffi/PROTOCOL.md` — Contrat ABI transport actuel
- `packages/rust/armonik-transport-ffi/STATE_OF_THE_ART.md` — État de l'art FFI asynchrones
- `packages/rust/armonik-transport-ffi/BASELINE_COMPARISON.md` — Comparaison FFI actuelle / spike C#
- `packages/rust/armonik-transport-ffi/include/NativeMethods.g.cs` — Surface ABI v1 actuelle
- `packages/rust/armonik-transport-ffi/include/http_config.schema.json` — Schéma config actuel

---

## Prochaines étapes

- **DESIGN.MD** — Finalisation des API Rust et C, types exacts, contrats d'erreur, spécification
  TLA+
- **TASKS.MD** — Plan d'implémentation ordonné (le contrat public de `armonik-transport` étant un
  prérequis identifié)
