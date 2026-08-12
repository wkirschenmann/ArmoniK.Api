# Towards ArmoniK gRPC

## Statut du document

Ce document est une **synthèse de transition**, destinée à préparer une spec propre à partir de la
stack de PR actuelle. Il enregistre les décisions et contraintes acquises, identifie les points qui
doivent encore être spécifiés et propose une trajectoire de remédiation. Il n’est ni une ABI
stabilisée, ni un ordre de merge, ni un remplacement immédiat de `DESIGN.md`, `PROTOCOL.md` ou
`PR_REMEDIATION.md`.

Aucune PR ne doit être mergée dans `main` sur la seule base de ce document. La stack complète doit
d’abord être validée fonctionnellement, notamment sous .NET Framework 4.7.2 et 4.8, avec shutdown
et unload dynamique sûrs.

## 1. Décision directrice

La cible est un channel gRPC natif Rust commun, projeté vers les langages hôtes par une FFI mince :

```text
armonik-transport
        ↓
armonik-grpc-channel
        ↓
armonik-grpc-channel-ffi
        ↓
bindings idiomatiques, dont un CallInvoker .NET
```

Les trois crates ont des responsabilités distinctes :

- `armonik-transport` atteint un endpoint : DNS, TCP, TLS/mTLS, proxy et options socket ;
- `armonik-grpc-channel` parle HTTP/2 et gRPC : channel, calls, messages, metadata, status,
  trailers, deadlines, retry, flow control et cancellation ;
- `armonik-grpc-channel-ffi` possède l’isolation foreign : runtime Tokio, handles, callbacks,
  registries, task tracking, shutdown, callback barrier et unload.

Le comportement gRPC natif devient la source normative commune. Les bindings ne reconstruisent pas
chacun retry, framing, status, flow control ou lifecycle. Le client Rust peut consommer le même
channel via un adapter compatible avec ses stubs Tonic ; le binding .NET le consomme via un
`Grpc.Core.CallInvoker`.

Le terme **runtime** est réservé à l’executor ou au runtime d’isolation FFI. Le moteur fonctionnel
est un **channel gRPC**, pas un « transport runtime ».
## 2. Motivations et constats de départ

### 2.1 État de `armonik-transport`

`armonik-transport` fournit déjà les fondations réseau utiles :

- configuration et résolution de l’endpoint/origin ;
- TCP et options socket ;
- TLS, mTLS, CA explicite et racines natives ;
- override du nom vérifié par TLS ;
- proxy HTTP `CONNECT` ;
- connect timeout ;
- un `Connector` consommable par Hyper ou Tonic.

Il ne doit pas devenir le propriétaire de la sémantique gRPC. Il ne doit donc pas définir les
messages gRPC, les cardinalités, `grpc-status`, les retries gRPC, les deadlines distantes ou le
lifecycle d’une call.

Certaines options sont aujourd’hui déclarées dans `armonik-transport` alors qu’elles sont appliquées
au-dessus du connecteur. La migration doit notamment réexaminer `RetryConfig`, `Http2Config`, le
request timeout, le rate limiting et les options de pool. Une façade de configuration composée peut
préserver le vocabulaire public sans préserver une mauvaise frontière interne.

### 2.2 État du client .NET

`ArmoniK.Api.Client` cible `netstandard2.0` et doit rester utilisable sous .NET Framework 4.7.2 et
4.8. Le channel actuel :

- utilise `WinHttpHandler` lorsque possible sous .NET Framework ;
- retombe sur `GrpcWebHandler(WinHttpHandler)` lorsque le HTTP/2 natif de la plateforme échoue ;
- configure TLS, CA, mTLS, proxy et credentials Basic ;
- configure une policy de retry globale ;
- expose keepalive, idle timeout et reuse port ;
- transmet la cancellation gRPC ;
- supporte les quatre cardinalités présentes dans les services ArmoniK.

Deux options exposées sont actuellement non effectives :

- `RequestTimeout` ne pose aucune deadline et ne produit qu’un warning ;
- `OverrideTargetName` ne modifie pas le target name et ne produit qu’un warning.

Le channel natif doit rendre ces options effectives ou les refuser explicitement. Il ne doit pas
reproduire un fallback gRPC-Web uniquement nécessaire à cause des limitations de WinHTTP : Hyper
peut parler HTTP/2 directement sur les versions de Windows ciblées. Un éventuel support gRPC-Web
serait une capability séparée, justifiée par un serveur qui l’exige.

### 2.3 État de la FFI expérimentale

La FFI existante a démontré des mécanismes importants : client Hyper lazy, body duplex, lecture et
écriture armées, flow control, trailers, cancellation, timeout, rate permit et callback terminal.
Elle révèle aussi les points qui doivent être fondés avant stabilisation :

- runtime Tokio process-wide, lazy et non joignable ;
- registries globales plutôt que scopées par runtime ;
- absence de start gate et de task group complet ;
- absence de callback barrier globale ;
- release/cancel sans preuve de join de la task ;
- risque de callback très rapide avant publication complète du state host ;
- identité pointer-shaped vulnérable à l’ABA après réutilisation d’adresse ;
- impossibilité de prouver un unload dynamique sûr.

Le reactor et ses tests sont une source à conserver. Leur couplage actuel à C, aux raw pointers et
au callback contract n’est pas une frontière à figer.

## 3. Principes d’architecture

1. **Une seule sémantique gRPC.** Le channel Rust définit le framing, les metadata, le status, les
   cardinalités, les deadlines, le retry et la cancellation.
2. **Un connecteur reste un connecteur.** DNS, TCP, TLS/mTLS et proxy restent sous le channel.
3. **Pas de runtime implicite dans le cœur.** `armonik-grpc-channel` utilise un executor fourni ou un
   adapter explicite ; il ne crée pas de runtime process-wide.
4. **Le runtime FFI possède tout travail foreign.** Toute task démarrée pour une ressource FFI est
   enregistrée, annulable et joignable par son runtime parent.
5. **Configuration n’est pas connexion.** Créer un channel valide et copie sa configuration, mais
   n’ouvre pas nécessairement de socket ; la première connexion physique peut rester lazy.
6. **Un channel logique n’est pas une socket.** Il peut sélectionner, recréer ou partager plusieurs
   connexions HTTP/2 physiques compatibles.
7. **Les providers sont natifs Rust par défaut.** CA système, DNS, proxy, credentials connus et
   diagnostics ne sont pas des host-calls ordinaires.
8. **Les callbacks ciblent un trampoline officiel.** Aucun user code, aucune continuation et aucun
   export réseau ne s’exécutent sur un worker Tokio.
9. **Pas d’extraction générique prématurée.** Le runtime d’isolation reste dans
   `armonik-grpc-channel-ffi` jusqu’à ce qu’une deuxième FFI réelle démontre la partie générique.
10. **Safety avant liveness.** Un timeout de shutdown ne justifie jamais une libération ou un unload
    non quiescent.

## 4. Responsabilités des crates

### 4.1 `armonik-transport`

Responsabilités cibles :

- endpoint à joindre et origin logique ;
- DNS et établissement TCP ;
- TLS, mTLS, racines système et CA explicite ;
- server-name override ;
- proxy HTTP `CONNECT` et authentification proxy ;
- connect timeout ;
- options TCP réellement applicables par le connecteur ;
- erreurs réseau structurées sans secret dans les messages ou `Debug`.

Hors responsabilité :

- client ou pool Hyper ;
- framing gRPC ;
- cardinalités et messages ;
- status/trailers gRPC ;
- retry gRPC ;
- deadline de call ;
- callback C et handles foreign ;
- runtime Tokio possédé.

### 4.2 `armonik-grpc-channel`

Responsabilités cibles :

- client/pool Hyper HTTP/2 et connexions physiques lazy ;
- framing et déframing des messages gRPC ;
- unary, client streaming, server streaming et bidirectional streaming ;
- request/response metadata, initial metadata, status et trailing metadata ;
- flow control et demande de lecture bornée ;
- half-close du request stream ;
- cancellation locale et propagation du reset ;
- deadline locale et header `grpc-timeout` cohérent ;
- retry/backoff et notion de call committed ;
- options HTTP/2, pool, keepalive et limites de headers/messages ;
- une API fonctionnelle Rust sans notion de C ABI ;
- un adapter utilisable par `armonik::Client<T>` et les stubs Tonic ;
- instrumentation future indépendante du binding.

Le crate peut dépendre de Tokio et Hyper comme implémentation async, mais il ne doit pas posséder un
singleton Tokio. Toute task de fond doit passer par un executor abstrait ou un adapter que le
runtime FFI peut tracker. Un consumer Rust ordinaire fournit son adapter Tokio ; la FFI fournit un
adapter enregistré dans son task group.

### 4.3 `armonik-grpc-channel-ffi`

Responsabilités cibles :

- ABI C versionnée ;
- `ak_runtime`, `ak_channel`, `ak_call` et éventuels handles de payload ;
- runtime Tokio explicitement possédé et joignable ;
- registries scopées par runtime ;
- IDs generation-tagged ou représentation équivalente ;
- validation owner/generation/kind avant toute dereference ;
- copie des inputs borrowed avant retour ;
- callbacks, trampoline contract et callback barrier ;
- task group, start gate, cancellation et shutdown ;
- conversion entre types ABI et types fonctionnels ;
- containment des panics et fautes de protocole ;
- preuve de quiescence avant `AK_RELEASED` et unload ;
- futur exporter de télémétrie vers le host.

Ce crate ne doit pas réimplémenter le framing, le retry ou le flow control gRPC.

## 5. Périmètre fonctionnel V1

La V1 est définie par les capacités réellement nécessaires au client ArmoniK sous .NET Framework
4.7.2/4.8, corrigées lorsque l’option actuelle est trompeuse.

### 5.1 Inclus

- endpoint HTTP et HTTPS ;
- HTTP/2 gRPC natif ;
- les quatre cardinalités gRPC ;
- metadata initiales et trailing metadata ;
- status gRPC et message de status ;
- cancellation ;
- deadline réellement appliquée et transmise au serveur ;
- retry avec les defaults ArmoniK actuels : cinq attempts au total, backoff initial 1 s, maximum
  5 s, multiplicateur 1,5, codes `UNAVAILABLE`, `ABORTED` et `UNKNOWN` ;
- TLS avec racines système ;
- CA PEM explicite ;
- connexion non vérifiée lorsque demandée explicitement ;
- mTLS depuis P12/PFX ou PEM plus clé ;
- `OverrideTargetName` effectif ;
- proxy désactivé, explicite, environnement et système Windows ;
- authentification proxy Basic ;
- connect timeout ;
- keepalive TCP, idle timeout et comportement de reuse port quand la plateforme le permet ;
- logger/diagnostics de base sans télémétrie OpenTelemetry ;
- création synchrone de configuration et connexion physique lazy ;
- shutdown déterministe et unload dynamique sûr.

### 5.2 Hors V1

- télémétrie OpenTelemetry/OTLP ;
- hedging ;
- load balancing générique et service config extensible ;
- resolver plugins ;
- compression configurable ;
- rate limiting public configurable s’il n’est pas requis par le client de référence ;
- interceptors génériques ;
- channelz public ;
- API publique de connectivity state ;
- provider host arbitraire ;
- runtime FFI générique partagé par plusieurs domaines ;
- gRPC-Web, sauf exigence serveur démontrée.

Le fait qu’une capacité soit hors V1 n’autorise pas l’ABI à empêcher son ajout. Les records
évolutifs utilisent `size`, `version`, `flags` et des champs réservés validés à zéro.

## 6. Configuration et providers

### 6.1 Configuration composée

La configuration publique du channel doit composer :

```text
GrpcChannelConfig
├── TransportConfig
│   ├── endpoint
│   ├── TLS/mTLS
│   ├── TCP
│   ├── proxy
│   └── connect timeout
├── Http2Config
├── RetryConfig
├── deadline/request timeout
├── pool options
└── user agent
```

La migration doit préserver autant que possible les noms d’options déjà consommés par les clients,
mais leur type propriétaire doit suivre leur responsabilité. En particulier, `RetryConfig` et les
options qui pilotent Hyper ou une call appartiennent conceptuellement à `armonik-grpc-channel`.

#### Propriété, consommation et durée de vie

`GrpcChannelConfig` est l’agrégat public défini par `armonik-grpc-channel`. Il compose le
`TransportConfig` défini par `armonik-transport` avec les politiques propres au channel. Être
propriétaire d’un type signifie définir sa sémantique, ses defaults et sa validation ; cela ne
signifie pas nécessairement être le seul composant à recevoir une valeur dérivée de cette
configuration.

| Configuration | Propriétaire sémantique | Consommateur effectif | Moment de consommation | Valeur conservée par |
|---|---|---|---|---|
| `GrpcChannelConfig` | `armonik-grpc-channel` | builder du channel, qui distribue chaque sous-configuration | création du channel | channel logique, sous forme normalisée et immuable |
| `TransportConfig.endpoint` | `armonik-transport` | résolution de la cible réseau, de l’origin et du nom TLS ; le channel ne reçoit ensuite que la cible et l’authority résolues | création du connecteur, puis chaque connexion physique utilise la cible résolue | connecteur transport et channel logique |
| TLS/mTLS | `armonik-transport` | loader de certificats et builder rustls ; handshake lors d’une connexion physique | fichiers lus à la création, policy appliquée à chaque connexion | connecteur TLS, avec secrets/certificats owned |
| TCP | `armonik-transport` | `HttpConnector` et socket physique | construction du connecteur puis ouverture de chaque socket | connecteur transport |
| proxy | `armonik-transport` | resolver de route puis `ProxyConnector` | route fixe préparée à la création ; environnement ou PAC résolu selon la policy au moment documenté | connecteur et éventuel cache du resolver |
| connect timeout | `armonik-transport` | dial TCP, résolution proxy et handshake `CONNECT` selon la borne définie | chaque tentative de connexion physique | connecteur transport |
| `Http2Config` | `armonik-grpc-channel` | builder Hyper HTTP/2 et gestionnaire de connexions | création du channel et établissement de chaque connexion HTTP/2 | channel/pool |
| `RetryConfig` | `armonik-grpc-channel` | coordinateur de call et d’attempts | snapshot au démarrage de la call, puis après chaque attempt retryable | call logique jusqu’à son terminal |
| deadline/request timeout | `armonik-grpc-channel` | coordinateur de call, timer et génération de `grpc-timeout` | calcul d’une deadline absolue au start ; une option par-call peut remplacer le default du channel | call logique et attempts dérivées |
| pool options | `armonik-grpc-channel` | pool Hyper et politique d’éviction/idle | création du channel, puis acquisition et restitution des connexions | channel/pool |
| user agent | `armonik-grpc-channel` | builder des metadata de chaque attempt | au démarrage de chaque attempt, comme valeur par défaut si la call n’en fournit pas | configuration du channel ; valeur matérialisée dans les headers de l’attempt |

L’interprétation d’une option n’est faite qu’à un niveau. Par exemple, `endpoint` produit une cible
de dial et une origin/authority ; `armonik-transport` effectue cette normalisation une seule fois,
puis `armonik-grpc-channel` utilise le résultat sans réinterpréter TLS ou `OverrideTargetName`.
Inversement, le transport ne lit jamais `RetryConfig` ou la deadline d’une call.

La configuration du runtime FFI est orthogonale et ne fait pas partie de `GrpcChannelConfig` :

```text
RuntimeConfig                      consommé par armonik-grpc-channel-ffi
├── executor threads               à ak_runtime_create
├── callback/trampoline            pendant toute la vie du runtime
├── queue et limites FFI           par le dispatcher et les callbacks
├── shutdown/status policy         à la fermeture
└── future telemetry sink          hors V1

GrpcChannelConfig                  consommé par armonik-grpc-channel
└── TransportConfig               délégué à armonik-transport

CallStartOptions                   consommé par armonik-grpc-channel
├── method et cardinality
├── metadata
├── deadline override
└── futur W3C parent context       copié par la FFI, interprété par l’instrumentation du channel
```

`armonik-grpc-channel-ffi` consomme seulement la **représentation ABI** : il valide les layouts,
copie les bytes borrowed, décode la configuration et appelle les builders typés. Il ne décide pas
des defaults réseau ou gRPC et ne réimplémente aucune option. De même, les bindings C#/Java/etc.
produisent la configuration et présentent les erreurs dans leur idiome ; ils n’appliquent ni retry,
ni deadline, ni proxy eux-mêmes.

Le schema public doit être généré depuis cet agrégat composé. Chaque sous-crate reste la source de
vérité de ses champs, tandis que l’agrégat garantit l’unicité des noms et permet de vérifier qu’une
option est effectivement consommée. Un ledger de configuration doit associer chaque option à un
unique consommateur, un moment d’application et un test ; toute option ajoutée sans entrée dans ce
ledger doit faire échouer la validation.

Après création, la configuration du channel est immuable. Une call prend un snapshot des defaults
qui la concernent au start. Une future reconfiguration éventuelle créera une nouvelle génération de
policy pour les calls futures ; elle ne modifiera pas implicitement une call ou une connexion déjà
en cours.

### 6.2 Proxy système Windows

Le modèle cible distingue :

```rust
pub enum ProxySource {
    Disabled,
    Explicit(Uri),
    Environment,
    #[cfg(windows)]
    WindowsSystem,
}
```

Mapping à formaliser dans la spec :

- `none` vers `Disabled` ;
- URL vers `Explicit` ;
- `env` vers `Environment` ;
- `system` sous Windows vers `WindowsSystem` ;
- valeur vide vers le default plateforme documenté.

Sous Windows, `WindowsSystem` peut utiliser WinHTTP pour lire le proxy système et résoudre
PAC/WPAD. WinHTTP est seulement le resolver ; Hyper/rustls restent le moteur réseau. ArmoniK ne doit
pas embarquer son propre moteur PAC.

Une résolution PAC peut bloquer, télécharger et exécuter un script. Elle ne doit donc jamais être
faite dans `poll_ready`, un callback FFI ou sous le loader lock. Elle passe par une activité async ou
bloquante enregistrée dans le task group, avec timeout, cancellation et participation au shutdown.

### 6.3 Providers host

Le chemin normal reste natif Rust :

- racines de confiance plateforme ;
- DNS ;
- proxy ;
- credentials natifs connus ;
- diagnostics.

Des host-calls pourront être ajoutés plus tard pour un token lié à une session utilisateur, un
resolver métier, un certificate provider applicatif ou un proxy selector métier. Ils devront être
modélisés comme operations request/reply corrélées, jamais comme callback bloquant sur un worker
Tokio.

## 7. Modèle fonctionnel du channel et des calls

### 7.1 Channel logique

Le channel suit conceptuellement :

```text
Configured -> Open -> Draining -> Closed
```

`Configured` signifie que la configuration est validée et possédée ; aucune socket n’est promise.
`Draining` ferme le gate de nouvelles calls tout en laissant finir ou en annulant les calls actives
selon la policy. `Closed` signifie qu’aucune call ni task de connexion ne peut encore accéder au
channel.

Une call conserve une référence comptée sur son channel et sur les ressources transport nécessaires
jusqu’à sa quiescence native, même si le wrapper host a libéré sa propre référence.

### 7.2 Call gRPC

Une surface fonctionnelle conceptuelle est :

```text
Channel.open(method, cardinality, metadata, deadline) -> Call
Call.send_message(bytes) -> Future<SendAccepted>
Call.half_close() -> Future<Unit>
Call.initial_metadata() -> Future<Metadata>
Call.request_messages(n)
Call.next_message() -> Future<Message | End>
Call.cancel(reason)
Call.status() -> Future<StatusAndTrailers>
```

Le framing gRPC reste interne. Le binding échange des messages, pas des fragments HTTP/2. Une
future optimisation zero-copy ne change pas ce niveau de contrat.

États indicatifs :

```text
Created -> Started -> Active -> HalfClosed -> Terminal -> Quiescent
```

Les send et receive sides progressent indépendamment. `Terminal` fixe exactement une issue logique ;
`Quiescent` signifie qu’aucune task, callback, lease ou command en vol ne peut encore toucher la
call.

### 7.3 Retry

La spec devra définir précisément :

- ce qui rend une call committed ;
- quelles metadata ou messages doivent être bufferisés pour replay ;
- quelles cardinalités sont retryables ;
- l’interaction entre deadline globale, deadline d’attempt et backoff ;
- la course cancel/timeout/retry ;
- la limite de taille des données replayables.

Une call streaming ne doit jamais être rejouée aveuglément après publication de messages
applicatifs. La configuration globale actuelle du client .NET ne suffit pas à définir cette
sémantique ; le channel natif doit suivre une règle explicite et testée.

## 8. Modèle FFI

### 8.1 Ressources

La surface conceptuelle comporte :

```text
Runtime -> Channel -> Call
```

Le runtime possède les channels ; chaque channel et chaque call retient son parent jusqu’à
quiescence. Libérer une référence host ne détruit jamais un child encore actif.

Les handles sont opaques, thread-safe et validés dans une registry du runtime. Une adresse brute
n’est jamais une identity suffisante. Un handle stale, wrong-owner, wrong-kind ou wrong-generation
est rejeté sans dereference.

### 8.2 Inputs, outputs et payloads

- un input `ptr + len` est borrowed uniquement pendant le downcall ;
- Rust copie avant retour toute donnée utilisée ensuite ;
- un payload de callback est borrowed pendant le trampoline, sauf retain explicite ;
- un payload owned a un release symétrique par le même allocateur ;
- aucun heap host ne libère directement une allocation Rust ;
- toute taille, version, alignment, nullability, flag et overflow est validé avant lecture.

### 8.3 Callback contract

Le callback vise le trampoline officiel du binding :

```c
ak_callback_result on_event(
    void *runtime_context,
    ak_operation_token token,
    const ak_event *borrowed_event);
```

Le trampoline :

1. résout un token déjà publié ;
2. copie ou retain le payload ;
3. publie un completion record dans une queue host ;
4. signale le scheduler host ;
5. retourne sans exécuter de user code.

Le callback n’est pas invoqué inline par le downcall sur le thread appelant. Une completion très
rapide peut néanmoins être livrée concurremment avant que le downcall de start soit revenu. Le
binding doit donc allouer le token et publier tout state routable avant `call_start`. Si le start
échoue synchroniquement, le contrat doit garantir qu’aucun event ne sera ensuite émis pour ce token.

Les callbacks sont sérialisés par call, mais peuvent être concurrents entre calls. Aucun unwind,
exception foreign ou long jump ne traverse C.

### 8.4 Terminal et release

Chaque call acceptée produit exactement un terminal host, sous les assumptions de progress
déclarées. Aucun callback de cette call ne suit son terminal.

`call_release` libère la référence du caller et peut demander une cancellation ; il ne libère pas le
root callback et n’abort pas arbitrairement la task. Le root host reste vivant jusqu’au terminal et
à la fin effective du callback concerné.

## 9. Runtime FFI, shutdown et unload

### 9.1 Lifecycle observable

Le lifecycle est exposé par une fonction synchrone, thread-safe et non bloquante hors finalisation
bornée :

```c
ak_runtime_state ak_runtime_status(ak_runtime_handle runtime);
```

L’automate normal est monotone :

```text
AK_NOT_INITIALIZED
    -> AK_RUNNING
    -> AK_STOPPING
    -> AK_DRAINING
    -> AK_RELEASED
```

Une impossibilité de prouver la quiescence mène à :

```text
AK_STOPPING | AK_DRAINING -> AK_FAILED_UNQUIESCED
```

`AK_RELEASED` et `AK_FAILED_UNQUIESCED` sont terminaux. Une faute fonctionnelle qui atteint malgré
tout la quiescence est rapportée comme résultat du shutdown, mais le lifecycle peut continuer vers
`AK_RELEASED` : l’échec de l’opération et la sûreté de l’unload sont deux informations distinctes.

`AK_NOT_INITIALIZED` est la valeur d’un handle nul/zéro ou du wrapper host avant une création
réussie. Un token non nul mais inconnu, stale ou de mauvaise génération est `AK_INVALID_HANDLE`,
jamais `AK_NOT_INITIALIZED` ni `AK_RELEASED`.

Le runtime contient au minimum :

- un executor Tokio possédé ;
- un start gate ;
- les registries de handles et tokens ;
- un task group ;
- la configuration callback ;
- un compteur de callbacks in-flight ;
- une callback barrier ;
- un fault latch ;
- les channels, calls et providers enfants.

Une task est enregistrée avant son spawn observable. Fermer le start gate précède toute attente de
quiescence. Aucun child ne peut être créé une fois le gate fermé.

### 9.2 API de shutdown

La surface publique comporte trois fonctions de lifecycle :

```text
ak_runtime_create          synchrone, crée le runtime et le fait passer à AK_RUNNING
ak_runtime_begin_shutdown  opération asynchrone : ferme le start gate, draine et finalise
ak_runtime_status          synchrone, non bloquant, retourne l'état courant du runtime
```

`ak_runtime_begin_shutdown` est une **opération asynchrone** au même titre qu'un `call_start` :
elle retourne rapidement après avoir fermé atomiquement le start gate, puis le runtime effectue
l'ensemble du drain et de la finalisation en tâche de fond. Sa completion est délivrée au host par
le trampoline officiel. Cette completion est le **dernier callback** émis par ce runtime.

L'opération interne après fermeture du gate :

1. passe à `AK_STOPPING` et refuse les nouveaux children ;
2. ferme les channels et annule ou draine les calls selon la policy ;
3. passe à `AK_DRAINING` et attend tasks, providers, leases et callbacks ;
4. joint l'executor Tokio et le coordinator de shutdown ;
5. détruit l'objet runtime interne ;
6. publie la completion de shutdown par le trampoline — **dernier callback du runtime**.

`ak_runtime_begin_shutdown` est idempotent. Un second appel sur un runtime déjà au-delà de
`AK_RUNNING` est un no-op.

Si une faute rend la quiescence impossible, le runtime passe à `AK_FAILED_UNQUIESCED` et publie
une completion d'échec. Le host ne peut alors pas unload.

#### Fenêtre entre la completion et `AK_RELEASED`

La completion de shutdown arrive dans le trampoline, sur un thread qui exécute encore du code de la
DLL. Tant que ce thread n'est pas sorti du trampoline, le runtime ne peut pas passer à
`AK_RELEASED` : il resterait un thread en vol dans la bibliothèque et l'unload serait dangereux.

La séquence temporelle exacte est :

```text
    drain terminé
    → finalisation interne achevée
    → callback de shutdown délivré au trampoline
    → trampoline copie/retain, publie dans la queue host et retourne
    → le thread sort du code de la DLL
    → le runtime passe atomiquement à AK_RELEASED
```

Le passage à `AK_RELEASED` est effectué par le runtime lui-même après retour du trampoline —
c'est la dernière instruction exécutée par le dernier thread natif de ce runtime. Le binding n'a
pas à déclencher cette transition ; il la constate en pollant `ak_runtime_status`.

#### Pattern de consommation côté binding

Le binding expose naturellement :

```text
ShutdownAsync
    = begin_shutdown (retourne, ferme le gate)
    → await shutdown completion (callback normal via le trampoline)
    → poller ak_runtime_status jusqu'à AK_RELEASED
    → réussir
```

Le poll après réception de la completion couvre la fenêtre courte où le thread qui a invoqué le
trampoline finit de sortir du code natif. En pratique ce sont quelques instructions atomiques, mais
la fenêtre doit être modélisée. Une busy loop bornée ou un `yield`/`SpinWait` entre deux polls
suffit ; ce n'est pas une attente d'I/O.

Un timeout du `ShutdownAsync` du binding arrête seulement l'attente du caller. Il ne fabrique pas
`AK_RELEASED`, n'abandonne pas les roots et n'autorise pas l'unload. Un finalizer peut demander
le shutdown, mais ne force jamais le déchargement.

### 9.3 Témoin de statut et condition d'unload

La règle exposée au host est :

> L'unload est autorisé si et seulement si `ak_runtime_status(runtime) == AK_RELEASED` pour
> chaque runtime créé depuis cette instance de la DLL, et après retour de cet appel.

`AK_RELEASED` certifie que le start gate est fermé, les tasks et callbacks sont terminés, les
children, leases et payloads natifs sont libérés, les registries et providers sont fermés,
l'executor et le coordinator sont joints, l'objet interne est détruit et **aucun thread n'exécute
plus de code de la DLL pour ce runtime**.

Le handle consultable après destruction ne peut pas être un pointer vers l'objet runtime libéré. Il
est un **témoin de lifecycle stable** — token générationnel plus cellule de statut ou
représentation équivalente — dont seule la partie lourde est détruite avant le passage à
`AK_RELEASED`.

Cette exigence déplace un problème d'ownership qui devra être résolu explicitement dans la spec :
le témoin doit rester consultable jusqu'à l'unload sans produire une allocation heap perdue pour
chaque runtime créé. Une cellule fournie/possédée par le binding ou une table de slots bornée
appartenant à l'instance chargée sont des options ; un tombstone heap non libérable et un slot
réutilisé alors qu'un ancien token reste observable ne le sont pas.

Cette règle suppose également qu'aucun singleton process-wide, task globale ou ressource native
n'existe hors d'un `ak_runtime`. Sinon aucun statut individuel ne pourrait certifier l'unload de
la bibliothèque.

## 10. Binding .NET de référence

Le binding .NET implémente `Grpc.Core.CallInvoker`, ce qui permet de réutiliser les stubs C# générés
sans réécrire les clients de service. Il doit couvrir les cinq méthodes de `CallInvoker`, soit le
unary bloquant et les quatre formes async.

Le binding contient en interne :

- chargement/versionnement de la DLL ;
- `SafeHandle` ou ownership équivalent ;
- delegates rooted jusqu’à la callback barrier ;
- registry de tokens ;
- `ConcurrentQueue<CompletionRecord>` ou mécanisme équivalent ;
- signal du dispatcher ;
- `TaskCompletionSource` dont les continuations ne s’exécutent pas inline ;
- adaptation vers `AsyncUnaryCall`, streams reader/writer et status ;
- cancellation registration ;
- shutdown explicite avant unload.

La V1 ne doit pas dépendre de `LibraryImport` ou d’API indisponibles sous `netstandard2.0`.
`DllImport`, delegates et calling conventions explicites constituent le chemin de référence.

Java, Python et C++ pourront projeter la même ABI, mais leurs schedulers et lifecycles nécessiteront
des refinements propres. Ils ne sont pas une condition d’implémentation de la première V1 .NET,
mais l’ABI ne doit pas intégrer une convention exclusivement .NET.

## 11. Obligations TLA+

Le modèle ne sert pas à prouver la performance ni le progress d’un peer arbitraire. Il doit séparer
les safety properties, qui ne dépendent jamais de fairness, des liveness properties conditionnelles.

### 11.1 Spec fonctionnelle abstraite

Les modules fonctionnels ne sont pas quatre implémentations indépendantes à raffiner une par une.
Ils séparent les dimensions d’un même modèle abstrait afin de garder des variables et invariants
lisibles :

```text
GrpcChannel.tla       channel logique, calls et fermeture
CallLifecycle.tla     start, half-close, terminal et quiescence
GrpcFlowControl.tla   message demand, send admission et bornes
Retry.tla             attempts, commitment, backoff et deadline
AbstractGrpc.tla      composition des quatre modules en une spec observable
```

`AbstractGrpc.tla` définit les observations promises à un consumer : calls acceptées ou refusées,
messages admis, metadata, terminal, cancellation, retry et fermeture. Il ignore les handles C, les
queues, les callbacks, les threads et les roots .NET. C’est cette **composition abstraite** qui est
la cible du premier refinement.

Chaque module peut être model-checké isolément pour ses invariants locaux, mais la preuve de contrat
porte aussi sur `AbstractGrpc.tla`, où leurs interactions sont visibles : retry contre deadline,
half-close contre cancel, channel draining contre nouveau start, par exemple.

### 11.2 Implémentation FFI et premier refinement

Les modules FFI décrivent l’état concret ajouté par le lowering C :

```text
FfiRuntime.tla          gates, children, tasks, callbacks et shutdown
GenerationalIds.tla     allocation, validation, tombstones et non-confusion
CallbackProtocol.tla    publication, trampoline, résultat et barrier
HostQueue.tla           publish-before-signal, capacité et consommation
MemoryOwnership.tla     borrowed, copied, retained et released
FailureContainment.tla  fault latch, quarantine et quiescence
FfiGrpc.tla             composition concrète de ces modules avec les calls gRPC
FfiRefinement.tla       mapping de FfiGrpc vers AbstractGrpc
```

Le refinement n’est pas une correspondance 1:1 entre fichiers de 11.1 et de 11.2. Un même mécanisme
concret peut raffiner plusieurs transitions fonctionnelles ; inversement, plusieurs modules
concrets coopèrent pour raffiner un seul terminal abstrait.

`FfiRefinement.tla` définit explicitement le mapping :

- un `ak_call` validé et son token représentent une call abstraite ;
- commands et events représentent send, demand, half-close, cancel et terminal ;
- le channel/runtime start gate représente l’acceptation ou le refus d’un start ;
- la transition observée vers `AK_RELEASED` représente la fermeture abstraite et la destruction
  sûre ;
- allocation de slot, retain/release, enqueue/dequeue ou callback enter/leave sont généralement des
  pas de **stuttering** au niveau fonctionnel.

`GenerationalIds.tla`, `MemoryOwnership.tla` et une partie de `FailureContainment.tla` ajoutent aussi
des invariants de safety qui n’ont pas d’observation fonctionnelle correspondante. Ils renforcent
l’implémentation concrète ; ils ne doivent pas être artificiellement transformés en opérations
gRPC abstraites.

### 11.3 Binding .NET et second refinement

Le binding constitue un niveau concret supplémentaire :

```text
DotNetBinding.tla       roots, queue, TCS, cancellation, scheduler et dispose
DotNetComposition.tla   binding .NET composé avec le protocole FFI
DotNetRefinement.tla    mapping vers FfiGrpc et observations AbstractGrpc
Composition.tla         décharge des guarantees/assumptions entre les trois niveaux
```

La relation recherchée est :

```text
DotNetComposition
    refines FfiGrpc
        refines AbstractGrpc
```

Le premier mapping vérifie notamment qu’une callback native devient exactement une publication
host, que les roots vivent assez longtemps et que les continuations ne s’exécutent pas inline. Par
transitivité, les futures et streams vus par l’application respectent alors le contrat fonctionnel
abstrait. Les autres bindings auront leur propre second refinement sans recopier
`AbstractGrpc.tla` ni le modèle FFI commun.

`Composition.tla` rend visibles les assumptions déchargées par le niveau suivant. Par exemple, le
protocole FFI suppose un trampoline borné ; `DotNetBinding.tla` doit le garantir. L’application voit
la guarantee composée, pas l’assumption intermédiaire.

### 11.4 Safety properties minimales

- aucun start après fermeture du gate ;
- toute task est enregistrée avant spawn ;
- aucun callback après la callback barrier ;
- aucun token stale/wrong-owner/wrong-generation n’est dereferenced ;
- tout input utilisé après retour a été copié ou retained ;
- un terminal exactement au plus une fois ;
- aucun event après terminal ;
- un root host survit à tous ses callbacks ;
- un child retient son parent jusqu’à quiescence ;
- aucune release par le mauvais allocateur ;
- `ak_runtime_status == AK_RELEASED` implique tasks, callbacks, children et leases vides,
  registries et providers fermés, executor et coordinator joints ;
- aucun callback ou thread natif ne réapparaît après observation de `AK_RELEASED` ;
- `AK_FAILED_UNQUIESCED` et tout autre état interdisent l’unload ;
- le témoin de statut reste valide après destruction du runtime interne sans pointer vers sa mémoire
  ni produire une fuite non bornée ;
- les transitions de shutdown sont monotones ;
- aucune faute de routing ne livre un payload au mauvais consumer.

### 11.5 Liveness conditionnelle

Les propriétés de progress peuvent supposer :

- scheduler Rust et host finalement exécutés ;
- callback trampoline borné ;
- queue host finalement drainée ;
- peer qui répond, ferme ou atteint un timeout ;
- cancellation coopérative des providers ;
- user code que l’API décide d’attendre finit par rendre la main.

Une assumption non satisfaite peut produire un timeout ou `AK_FAILED_UNQUIESCED`, jamais une
violation de safety masquée par une libération forcée.

## 12. Télémétrie future, explicitement hors V1

### 12.1 Parenté entrante

OTLP est un protocole d’export, pas un contexte de parenté. Le binding capture le contexte W3C du
host au démarrage exact de la call :

```text
traceparent
tracestate optionnel
```

Sous .NET, il provient de `Activity.Current` seulement si son `IdFormat` est W3C. Le contexte est
capturé une fois pour une call streaming, copié pendant `call_start`, puis utilisé par Rust pour
créer le span natif enfant.

Chaîne attendue :

```text
host span H
└── native call span C
    ├── attempt span A1
    │   └── server span S1
    └── attempt span A2
        └── server span S2
```

Le contexte injecté dans chaque requête réseau est celui du span d’attempt lorsqu’il existe, sinon
celui du span de call. Réinjecter directement H ferait du serveur un sibling du span natif.

Le baggage n’appartient pas au premier contrat. Un Activity hiérarchique n’est pas converti
artificiellement en W3C.

### 12.2 Télémétrie sortante contrôlée par le host

L’application hôte doit pouvoir récupérer **toute la télémétrie native** et choisir sa destination.
L’export OTLP direct depuis Rust n’est donc pas le chemin normatif.

```text
OpenTelemetry SDK Rust
        ↓ exporter FFI
batches OTLP Protobuf
        ↓ trampoline + queue host
application hôte
        ├── collector OTLP choisi
        ├── transformation propriétaire
        ├── fichier ou diagnostic local
        └── plusieurs destinations
```

Le format recommandé est le payload Protobuf non transporté de
`ExportTraceServiceRequest`, `ExportMetricsServiceRequest` ou `ExportLogsServiceRequest`, avec un
discriminant de signal et d’encoding. Resources, instrumentation scopes, spans, links, events,
metrics, exemplars et logs restent ainsi représentables sans inventer un modèle ABI propriétaire.

Le callback de télémétrie ne fait aucun export réseau. Il copie ou retain un batch, le publie dans
une queue host, signale le dispatcher et retourne. L’acceptation signifie que le host possède le
batch, pas qu’un backend externe l’a reçu.

### 12.3 Backpressure et pertes

« Toute la télémétrie » ne peut pas garantir simultanément mémoire bornée, absence de perte, progress
des calls et host autorisé à ne jamais consommer. La future spec doit offrir une policy explicite :

- `LosslessWhileHostDrains` : aucune perte tant que le host progresse ; saturation et shutdown
  peuvent attendre, sans bloquer les tasks réseau ;
- `BestEffort` : les calls ne sont jamais retardées, mais la saturation peut drop avec compteurs
  observables.

Une garantie inconditionnelle demanderait un spool durable, hors de ce design initial. Les retries
de livraison FFI donnent au mieux une sémantique at-least-once ; un sequence ID stable permet la
déduplication.

### 12.4 Shutdown télémétrie

Avant unload :

```text
terminer les calls et spans
-> collecter les derniers signaux
-> force_flush du SDK Rust
-> remettre les batches au host
-> attendre les callbacks télémétriques
-> arrêter l’exporter FFI
-> joindre ses workers
-> franchir la callback barrier
```

Une fois un batch copié ou retained par le host, son export peut continuer après l’unload, car il
n’appartient plus au domaine mémoire ou au code de la DLL.

Un mode `DirectOtlp` pourra être ajouté comme commodité explicite. Il ne remplace pas `HostSink` et
ne choisit jamais silencieusement endpoint, credentials, proxy ou destination à la place du host.

## 13. Trajectoire depuis la stack de PR actuelle

Ce document ne prescrit ni fermeture ni merge automatique. Il propose de replacer chaque invariant
dans la première PR qui crée l’objet concerné.

| Élément actuel | Trajectoire proposée |
|---|---|
| #743, connector transport | conserver comme fondation réseau ; maintenir la frontière connector/channel |
| #744, skeleton ABI et runtime global | conserver les guards, buffers, errors et génération ; ajouter `ak_runtime`, statut observable, registries scopées, IDs générationnels, task group, shutdown, join interne et callback barrier |
| #745, génération d’options | conserver hors du chemin critique ; générer des données, pas une state machine async |
| #746, reexports pour laisser la FFI construire Hyper | reformuler si possible en fondation de `armonik-grpc-channel`; remplacer son intention si son historique ne correspond plus au diff |
| #747, client/pool Hyper dans la FFI | déplacer la machine fonctionnelle dans `armonik-grpc-channel`; garder dans la FFI uniquement conversion, handles et parenté runtime/channel |
| branche reactor | reprendre state machine et tests par morceaux ; séparer core gRPC et lowering callback FFI |

Ordre de travail indicatif pour la future spec :

1. stabiliser les responsabilités et la configuration des trois crates ;
2. extraire le moteur fonctionnel vers `armonik-grpc-channel` sans C ABI ;
3. rendre ce channel consommable en Rust via un adapter Tonic ;
4. amender la fondation FFI avec runtime possédé, IDs générationnels et shutdown ;
5. projeter channel et call dans l’ABI ;
6. reprendre le reactor comme lowering de la machine fonctionnelle ;
7. construire le `CallInvoker` .NET de référence ;
8. exécuter les tests de stack complète et d’unload ;
9. seulement après validation, décider l’ordre final de rebase et de merge.

Les documents existants qui parlent de `armonik-transport::runtime`, d’une ABI HTTP/2 sans gRPC ou
d’un runtime process-wide devront être réconciliés avec la spec finale. Ils restent utiles comme
historique des invariants découverts, pas comme raison de conserver la frontière initiale.

## 14. Validation attendue avant stabilisation

### 14.1 Core Rust

- configuration sans ouverture de socket ;
- première connexion lazy ;
- quatre cardinalités contre un serveur réel ;
- metadata, trailers-only et status non-OK ;
- gros messages au-delà de la fenêtre HTTP/2 ;
- slow consumer et flow control borné ;
- cancellation avant/après headers ;
- timeout avant/après headers ;
- half-close et races terminales ;
- retries et commitment par cardinalité ;
- plusieurs calls concurrentes sur un channel ;
- fermeture du channel pendant des calls actives ;
- proxy explicite, environnement et Windows system ;
- TLS, CA, mTLS et target override.

### 14.2 FFI

- callback très rapide avant retour du start ;
- callback sérialisé par call et concurrent entre calls ;
- unknown/stale/wrong-owner tokens ;
- duplicate terminal et event tardif détectés ;
- release concurrent avec callback/cancel/terminal ;
- inputs copiés avant retour ;
- queue saturation et scheduler rejection ;
- panic containment ;
- start refusé après fermeture du gate ;
- task group vide après shutdown ;
- callback barrier sans callback tardif ;
- timeout d’attente laissant le statut distinct de `AK_RELEASED`, sans free ni unload ;

### 14.3 .NET Framework

- tests net472 et net48 ;
- stubs générés utilisant le `CallInvoker` natif ;
- unary bloquant et async ;
- client, server et bidirectional streaming ;
- cancellation token et deadline ;
- continuations non-inline ;
- rooting des delegates sous GC forcé ;
- exceptions et scheduler rejection confinés ;
- dispose pendant call active ;
- cycles répétés create/start/begin-shutdown/status jusqu’à `AK_RELEASED` ;
- unload uniquement après quiescence démontrée.

### 14.4 Preuve et conformance

Chaque action normative importante doit être reliée à :

```text
action TLA+
    <-> transition ou check dans le code
    <-> test de conformance ou fault injection
```

Les traces de conformance doivent être communes au core, à la FFI et au binding, sans recopier la
machine fonctionnelle dans chaque modèle de langage.

## 15. Points à décider dans la future spec

1. API Rust publique exacte de `Channel` et `Call`.
2. Adapter executor permettant le tracking FFI sans dégrader l’usage Rust normal.
3. Sémantique exacte de pool et de channel logique.
4. Point de commitment et retry autorisé pour chaque cardinalité.
5. Bornes de message, metadata, buffers replayables et queues.
6. Mapping final des options existantes vers la configuration composée.
7. Default proxy vide sous Windows et hors Windows.
8. API et audit du resolver WinHTTP : timeout, cancellation et fermeture des handles.
9. Format exact des IDs générationnels et des operation tokens.
10. Garantie précise sur callback concurrent avant retour de `call_start`.
11. Policy de drain des calls lors de `channel_close` et `runtime_begin_shutdown`.
12. Statuts ABI et mapping complet vers `Grpc.Core.StatusCode`.
13. Packaging, RID, chargement et stratégie d’unload .NET Framework.
14. Limites entre diagnostics V1 et future télémétrie OpenTelemetry.
15. Policy future de télémétrie `HostSink`, saturation, ownership et flush.

## 16. Exit criteria architecturaux

La migration peut être considérée prête à stabiliser lorsque :

- les trois crates existent avec les responsabilités ci-dessus ;
- aucune sémantique gRPC n’est dupliquée dans la FFI ou le binding .NET ;
- le client Rust et le `CallInvoker` .NET passent par le même channel natif ;
- les options V1 ont un comportement effectif et testé ;
- tout objet FFI a owner, children, start gate, terminal et destroy condition explicites ;
- chaque task est trackée et joignable ;
- chaque callback root survit jusqu’à sa barrière ;
- aucun handle n’est validé par simple dereference d’adresse ;
- les propriétés de safety ne dépendent d’aucune fairness assumption ;
- les liveness assumptions résiduelles sont documentées ;
- seul `AK_RELEASED` certifie la quiescence et autorise l’unload ;
- tout autre statut interdit l’unload sans destruction forcée ;
- la stack complète fonctionne sous net472 et net48 ;
- le mapping TLA+ / code / tests est traçable ;
- aucun merge dans `main` n’a lieu avant cette validation fonctionnelle de bout en bout.

## 17. Références informatives

- [W3C Trace Context](https://www.w3.org/TR/trace-context/)
- [.NET `System.Diagnostics.Activity`](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.activity)
- [OpenTelemetry context propagation](https://opentelemetry.io/docs/concepts/context-propagation/)
- [WinHTTP AutoProxy API](https://learn.microsoft.com/en-us/windows/win32/winhttp/winhttp-autoproxy-api)
- [.NET `Grpc.Core.CallInvoker`](https://github.com/grpc/grpc-dotnet/blob/master/src/Grpc.Core.Api/CallInvoker.cs)

Content was rephrased for compliance with licensing restrictions.
