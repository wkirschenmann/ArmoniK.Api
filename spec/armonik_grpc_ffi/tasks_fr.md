# TASKS — Client .NET ArmoniK sur channel gRPC natif Rust

## Philosophie

On part de main. On construit le chemin le plus court vers un unary call .NET → serveur gRPC
réel. Chaque tâche est un commit fonctionnel qui ajoute une capacité testée. La stack de PR
existante (#711–#747) est une source de code à piocher, pas un prérequis à intégrer en bloc.

Le livrable est une stack PR fonctionnelle. Rien n'est mergé dans main avant validation
complète de bout en bout. Des petites PRs de bugfix directement sur main sont possibles.

La preuve TLA+ vient avant l'implémentation FFI.

---

## Contexte : PRs existantes

La stack actuelle (#711 → #747) construit `armonik-transport` incrémentalement. Le code est
bon mais organisé pour l'ancienne architecture (FFI transport HTTP/2, pas de couche gRPC).

**Ce qu'on pioche** : le code transport (proxy, TLS, serde, connector, tests) au moment où on
en a besoin. **Ce qu'on ne reprend pas tel quel** : l'organisation en stack empilée sur 18 PRs,
la FFI skeleton/client (#744–#747), les reexports (#746).

Chaque tâche ci-dessous indique si elle pioche dans la stack ou écrit from scratch.

---

## Phase 0 — TLA+ (avant toute implémentation FFI)

### T0.1 : Spec TLA+ niveau 0 (AbstractGrpc)

**Prérequis** : Aucun
**Commit** : Créer `spec/armonik_grpc_ffi/tla/AbstractGrpc.tla`
- Variables : runtime_state, channels, calls, 4 séquences (submitted/sent/received/delivered),
  events_delivered, send_closed
- Invariants de safety : MetadataFirst, StatusLast, NoEventAfterStatus, UniqueTerminal,
  SendAfterEndSend, MonotoneRuntime, SingleRuntime, ChannelOwnership, CallOwnership,
  CreateRequiresRunning, StoppingClosesChannels, ReleasedNoCalls, SubmittedPrefixOfSent,
  ReceivedPrefixOfDelivered, OrderPreserved
- Liveness : EventualTerminal, EventualShutdown, SubmitProgress, DeliveryProgress,
  EventualMessage

**Livrable** : Spec TLA+ explorable par TLC. Structure prête pour TLAPS.

### T0.2 : Spec TLA+ niveau 1 (FfiGrpc)

**Prérequis** : T0.1
**Commit** : Créer `spec/armonik_grpc_ffi/tla/FfiGrpc.tla`
- Variables ajoutées : handles, callbacks_in_flight, start_gate, at_ffi_boundary_send/recv
- Invariants : HandleValidity, BorrowedLifetime, CallbackSerialization, SingleSendInFlight,
  SingleRecvInFlight, GateClosed, ReleasedImpliesQuiescent, FfiBoundaryOrder
- Refinement mapping vers AbstractGrpc
- Décomposition des fairness niveau 0 en fairness locales

**Livrable** : Spec TLA+ niveau 1. Refinement vérifié par TLC.

### T0.3 : Spec TLA+ niveau 2 (DotNetBinding)

**Prérequis** : T0.2
**Commit** : Créer `spec/armonik_grpc_ffi/tla/DotNetBinding.tla`
- Variables : call_states (GCHandle), host_queue, tcs_state, gc_roots, dispose_state
- Invariants : RootSurvivesCallbacks, GCHandleAllocBeforeStart, ContinuationsAsync,
  DisposeAwaitsReleased
- Refinement mapping vers FfiGrpc
- Justification fairness "callback retourne" (trampoline borné)

**Livrable** : Spec TLA+ niveau 2. Refinement vérifié.

### T0.4 : Preuves TLAPS (3 niveaux)

**Prérequis** : T0.3
**Commit** : Prouver par TLAPS :
- Invariants de safety des 3 niveaux (induction)
- Refinements (simulation) : niveau 2 → niveau 1 → niveau 0
- Décomposition des fairness

**Livrable** : `spec/armonik_grpc_ffi/tla/proofs/`. TLAPS valide.

---

## Phase 1 — Unary call E2E minimal (le chemin le plus court)

Le minimum absolu pour un unary call : un connecteur HTTP/2 plain (pas de TLS, pas de proxy,
pas de retry), un framing gRPC, une FFI minimale, un binding .NET minimal.

### T1.1 : Créer `armonik-grpc-channel` — connecteur minimal + framing + unary

**Prérequis** : Aucun (parallélisable avec Phase 0)
**Source** : from scratch, en piochant le connector plain HTTP de la stack existante
**Commit** : Créer `packages/rust/armonik-grpc-channel/` :
- Dépendances : hyper, hyper-util, http, bytes, tokio
- `GrpcChannel::new(endpoint, executor)` — connecte en HTTP/2 plain (pas de TLS)
- Framing gRPC (encode/decode length-prefixed)
- `start_call(method, metadata)` → `GrpcCall`
- `GrpcCall` : send_message, end_send, next_message → RecvResult (Message | End(GrpcStatus))
- Executor trait

**Livrable** : Test d'intégration Rust : unary call vers un serveur gRPC local (plain HTTP/2).

### T1.2 : Créer `armonik-grpc-channel-ffi` — ABI minimale unary

**Prérequis** : T1.1, T0.2 (spec FFI prouvée ou au moins écrite)
**Source** : from scratch selon le design
**Commit** : Créer `packages/rust/armonik-grpc-channel-ffi/` :
- `ak_runtime_create`, `ak_runtime_status`, `ak_runtime_begin_shutdown`
- `ak_channel_create` (config JSON minimale : juste endpoint)
- `ak_call_start`, `ak_call_send_message` (zero-copy), `ak_call_end_send`,
  `ak_call_cancel`, `ak_call_release`, `ak_event_consumed`
- `ak_abi_version`
- Events : WRITE_DONE, INITIAL_METADATA, MESSAGE, STATUS, SHUTDOWN_COMPLETE
- SlotMap registry, runtime Tokio possédé, callback avec call_ctx
- Un seul send en vol, un seul event non-consumed par call

**Livrable** : Test C ou Rust FFI : unary call via l'ABI vers un serveur gRPC local.

### T1.3 : Créer `ArmoniK.Api.Client.RustGrpcChannel` — binding .NET unary

**Prérequis** : T1.2
**Source** : from scratch selon le design, patterns du spike (`wk/spike/ffi-http2-handler`)
**Commit** : Créer `packages/csharp/ArmoniK.Api.Client.RustGrpcChannel/` :
- P/Invoke pour tous les entry points (DllImport, Cdecl)
- NativeRuntime + NativeChannel (SafeHandle)
- Trampoline (delegate static, GCHandle runtime_ctx)
- HostQueue + Dispatcher
- CallState (GCHandle call_ctx, TCS pour metadata/status, Channel<> pour messages)
- NativeCallInvoker : BlockingUnaryCall + AsyncUnaryCall
- Pin buffer pour send zero-copy, attente WRITE_DONE, unpin
- Cancellation via CancellationToken → ak_call_cancel

**Livrable** : Test E2E .NET : unary call vers un serveur gRPC local (plain HTTP/2).

### T1.4 : Intégrer dans `ArmoniK.Api.Client` — CallInvoker injectable

**Prérequis** : T1.3
**Commit** : Rendre le CallInvoker injectable dans le client existant.
Mapping minimal des options (juste endpoint pour l'instant).
Erreur explicite si DLL native absente.

**Livrable** : Un test client ArmoniK existant passe avec le CallInvoker natif (unary, plain HTTP).

---

## Phase 2 — Streaming (les 3 autres cardinalités)

### T2.1 : Client streaming (channel Rust + FFI + .NET)

**Prérequis** : T1.3
**Commit** : Plusieurs send_message avant end_send. Même mécanique FFI (WRITE_DONE par message).
Côté .NET : AsyncClientStreamingCall avec write stream.

**Livrable** : Test E2E .NET : client streaming.

### T2.2 : Server streaming (channel Rust + FFI + .NET)

**Prérequis** : T1.3
**Commit** : Boucle next_message / event_consumed jusqu'à RecvResult::End.
Côté .NET : AsyncServerStreamingCall avec read stream.

**Livrable** : Test E2E .NET : server streaming.

### T2.3 : Bidi streaming (channel Rust + FFI + .NET)

**Prérequis** : T2.1, T2.2
**Commit** : Send et recv concurrents. Côté .NET : AsyncDuplexStreamingCall.

**Livrable** : Test E2E .NET : bidi streaming.

---

## Phase 3 — TLS et connexion sécurisée

### T3.1 : TLS avec racines système

**Prérequis** : T1.1
**Source** : piocher le TLS config de la stack existante (#725, #726)
**Commit** : Ajouter TLS dans le connecteur (`CaSource::System`). Endpoint `https://`.

**Livrable** : Test : unary call en HTTPS avec CA système.

### T3.2 : CA PEM explicite

**Prérequis** : T3.1
**Source** : piocher de #726
**Commit** : `CaSource::PemFile`. Option dans le JSON config.

**Livrable** : Test : connexion avec CA custom.

### T3.3 : mTLS (PEM + PKCS12)

**Prérequis** : T3.2
**Source** : piocher de #726, #730
**Commit** : `IdentitySource::PemFiles` et `IdentitySource::Pkcs12`. Load + inject dans rustls.

**Livrable** : Tests mTLS : PEM pair et P12.

### T3.4 : OverrideTargetName effectif

**Prérequis** : T3.1
**Source** : piocher de `wk/fix/rust-override-target-server-name`
**Commit** : Override du ServerName dans le handshake rustls.

**Livrable** : Test : override target, handshake réussit avec un nom différent.

### T3.5 : WindowsStore (CA et identité client)

**Prérequis** : T3.3
**Commit** : `CaSource::WindowsStore` et `IdentitySource::WindowsStore`. Résolution côté Rust.

**Livrable** : Test (Windows CI) : mTLS depuis le store.

### T3.6 : Insecure (connexion non vérifiée)

**Prérequis** : T3.1
**Commit** : `CaSource::Insecure`. Opt-in explicite.

**Livrable** : Test : connexion sans vérification du certificat.

---

## Phase 4 — Proxy

### T4.1 : Proxy explicite

**Prérequis** : T1.1
**Source** : piocher #711, #712
**Commit** : `ProxySource::ExplicitUri` et `ExplicitWithCredentials`. CONNECT tunnel.

**Livrable** : Test : unary via proxy HTTP explicite.

### T4.2 : Proxy environnement

**Prérequis** : T4.1
**Source** : piocher #716
**Commit** : `ProxySource::Environment`. Lecture HTTP_PROXY/HTTPS_PROXY/NO_PROXY.

**Livrable** : Test : proxy via env.

### T4.3 : Proxy Windows system

**Prérequis** : T4.1
**Source** : code existant dans la stack
**Commit** : `ProxySource::WindowsSystem`. WinHTTP resolver async avec timeout.

**Livrable** : Test (Windows CI) : proxy système.

---

## Phase 5 — Retry, deadline, robustesse

### T5.1 : Deadline (locale + grpc-timeout)

**Prérequis** : T1.1
**Commit** : Timer local par call. Header `grpc-timeout` transmis au serveur. Expiration →
cancel + RecvResult::End(DEADLINE_EXCEEDED). Option default_deadline dans GrpcChannelConfig.

**Livrable** : Test : deadline expire → status DEADLINE_EXCEEDED.

### T5.2 : Retry automatique (unary)

**Prérequis** : T5.1
**Source** : piocher les types de #732 (RetryConfig)
**Commit** : RetryConfig dans GrpcChannelConfig. Backoff exponentiel. Codes retryable.
Unary retry transparent (pas de buffer nécessaire — un seul message, rejouable).

**Livrable** : Test : retry sur UNAVAILABLE, succès au 2ème attempt.

### T5.3 : Retry streaming (buffer de rejeu)

**Prérequis** : T5.2, T2.1
**Commit** : Buffer de rejeu configurable. Client streaming retryable si ≤ buffer.
Bidi retryable si pas de réponse reçue et ≤ buffer. Commitment detection.

**Livrable** : Tests : retry streaming ≤ buffer OK, retry streaming > buffer → committed (pas de retry).

### T5.4 : Shutdown complet du runtime FFI

**Prérequis** : T1.2, T0.4 (preuves safety)
**Commit** : begin_shutdown ferme le start gate. Draine/cancel les calls. Attend quiescence.
Join Tokio. Callback SHUTDOWN_COMPLETE. Poll status → RELEASED.

**Livrable** : Test : cycles create/shutdown/RELEASED. Pas de thread en vol après RELEASED.

### T5.5 : Connexion eager (option)

**Prérequis** : T1.1
**Commit** : Option `eager_connect` dans GrpcChannelConfig. Si true, connexion HTTP/2 au create.

**Livrable** : Test : avec eager, la connexion est établie avant le premier call.

### T5.6 : Keepalive TCP et idle timeout

**Prérequis** : T1.1
**Source** : piocher #741
**Commit** : Options keepalive et idle timeout dans la config. Appliqués au pool Hyper.

**Livrable** : Test : connexion idle est fermée après timeout.

---

## Phase 6 — Config, packaging, intégration finale

### T6.1 : Schéma JSON complet et génération C#

**Prérequis** : Toutes les options implémentées (T3.x, T4.x, T5.x)
**Source** : piocher #728, #745
**Commit** : Regénérer le schéma JSON depuis les types Rust finaux (schemars). Générer les
types C# (RustChannelOptions). Test de fraîcheur du schéma. Test round-trip C# → JSON → Rust.

**Livrable** : Schéma commité. Types C# générés. Pas de drift.

### T6.2 : Mapping complet des options existantes du client ArmoniK

**Prérequis** : T6.1, T1.4
**Commit** : Mapper toutes les options de `GrpcChannel` (ArmoniK.Api.Common.Options) vers
`RustChannelOptions` : endpoint, TLS, proxy, timeout, retry. Test de correspondance.

**Livrable** : Le provider natif accepte toutes les options du client existant.

### T6.3 : Build cross-platform et package NuGet

**Prérequis** : T1.2
**Commit** : CI build de la DLL native pour win-x64, win-x86, linux-x64, linux-x86, linux-arm64.
Package NuGet multi-RID avec `runtimes/{rid}/native/`. Résolution automatique.

**Livrable** : `dotnet pack` produit un NuGet fonctionnel. DLL résolue sur chaque plateforme.

### T6.4 : Tests E2E .NET Framework 4.7.2 et 4.8

**Prérequis** : T6.3, T2.3
**Commit** : Projet de test ciblant net472 et net48. Même suite que net6.0/net8.0.

**Livrable** : Tous les tests passent sur .NET Framework. Comportement identique.

### T6.5 : Campagne de benchmarks comparatifs

**Prérequis** : T6.4
**Commit** : Benchmarks latence unary (P50/P95/P99), throughput streaming, overhead mémoire.
Natif vs managé. net48 et net8.0. Résultats documentés.

**Livrable** : Baseline de performance établie.

### T6.6 : Documentation et nettoyage

**Prérequis** : T6.5
**Commit** : README, migration guide. Fermeture des PRs obsolètes de la stack. Nettoyage
du code mort.

**Livrable** : Repo propre. Stack PR prête à merger dans main.

---

## Phase 7 — Client Rust ArmoniK sur armonik-grpc-channel

### T7.1 : Adapter le client Rust ArmoniK pour utiliser armonik-grpc-channel

**Prérequis** : T1.1, T2.3 (channel Rust fonctionnel avec les 4 cardinalités)
**Commit** : Remplacer la dépendance Tonic directe du client Rust ArmoniK par un adapter
qui consomme `armonik-grpc-channel`. Les stubs Tonic générés fonctionnent via un `Channel`
adapter qui délègue à `GrpcChannel`. Le client Rust et le binding .NET partagent le même
moteur gRPC natif.

**Livrable** : Les tests du client Rust ArmoniK passent en utilisant `armonik-grpc-channel`
au lieu de Tonic directement. Même comportement fonctionnel.

---

## Graphe de dépendances

```text
                T0.1 → T0.2 → T0.3 → T0.4 (TLA+, parallèle à tout le reste)
                                        │
T1.1 ─────────────→ T1.2 ←─────────────┘ (FFI après preuve)
  │                    │
  │                  T1.3 → T1.4
  │                    │
  ├── T2.1, T2.2 → T2.3
  │
  ├── T3.1 → T3.2 → T3.3 → T3.4, T3.5, T3.6
  │
  ├── T4.1 → T4.2, T4.3
  │
  ├── T5.1 → T5.2 → T5.3
  │     T5.4, T5.5, T5.6
  │
  └── T6.1 → T6.2 → T6.3 → T6.4 → T6.5 → T6.6
```

---

## Parallélisation

- **Phase 0** (TLA+) avance en parallèle de la Phase 1 (code Rust)
- **T1.1** (channel Rust) n'a aucun prérequis bloquant — démarre immédiatement
- **T1.2** (FFI) attend T0.2 (spec FFI écrite) — la preuve complète (T0.4) est idéale mais la
  spec écrite suffit pour commencer l'implémentation
- **Phases 3, 4, 5** sont indépendantes entre elles — parallélisables après Phase 1
- **T6.3** (build cross-platform) peut démarrer dès T1.2
