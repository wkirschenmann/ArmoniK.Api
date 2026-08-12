# Design d’une isolation FFI asynchrone pour `armonik-transport`

## Statut

**Résumé.** Ce document enregistre le choix de travail retenu pour la V1 : une ABI callback
MsQuic-style vers un trampoline officiel de chaque binding. Il reste soumis à la review d’équipe et
ne stabilise pas encore l’ABI. La cible documentaire, après revue, est une suite
normative composée de `TRANSPORT.md`, `PROTOCOL.md` et de ce document : le premier fixe les
observations fonctionnelles, le second l’ABI, l’isolation et la description TLA+, le troisième les
contraintes d’architecture, les proof obligations et les décisions retenues. Les alternatives non
retenues resteront clairement marquées comme informatives.

**Intention.** Le message principal est de construire une frontière dont les quatre points de vue
— Rust fonctionnel, Rust FFI, runtime de binding et host fonctionnel — fournissent surtout des
garanties. Le design n’est acceptable que si leurs assumptions se déchargent par composition et si
les assumptions résiduelles sont générales, visibles et défendables.

Le design s’appuie sur l’[état de l’art](STATE_OF_THE_ART.md), sur la
[comparaison de la baseline](BASELINE_COMPARISON.md) et sur le
[plan de remédiation de la stack de PR](PR_REMEDIATION.md). La décision de V1 est :

1. séparer l’API fonctionnelle de transport du protocole technique d’isolation ;
2. conserver les appels réellement synchrones comme downcalls directs ;
3. utiliser un callback MsQuic-style vers un trampoline appartenant
   au runtime de binding, borné, sans user code et dont le retour est une garantie formalisée ;
4. conserver le runtime à completion queue/waitable comme alternative documentée, contre-modèle
   TLA+ et solution de repli déclenchée seulement si une cible invalide le trampoline ;
5. représenter les host calls bidirectionnels par request/reply corrélées ;
6. définir une quarantaine et une barrière de quiescence pour toute rupture d’invariant de routing ;
7. ne figer l’ABI qu’après modèles TLA+ commun et propres aux bindings, threat model, binding .NET
   Framework 4.8, binding Java 17 et benchmarks du chemin complet retenu.

Cette décision réutilise la forme déjà démontrée par la baseline tout en déplaçant le callback
applicatif hors de la frontière native. Chaque binding officiel doit garantir son trampoline ; si
une cible ne le peut pas, elle déclenche une réévaluation explicite du dispatcher natif ou du
waitable. La performance et la quantité de trusted code restent des conditions d’acceptation
mesurées, mais la V1 n’implémente pas deux protocoles complets par défaut.

## 1. Problème à résoudre

**Résumé.** `armonik-transport` expose une interaction HTTP/2 duplex, asynchrone et soumise au flow
control. La FFI doit préserver cette sémantique tout en isolant Rust des schedulers, GC, exceptions
et conventions de cancellation des langages cibles. Une simple génération de signatures ne peut
pas porter ce contrat.

**Intention.** Le lecteur doit retenir que le problème est la conservation d’une sémantique
concurrente et de ses lifetimes à travers plusieurs runtimes. La surface C n’est qu’un lowering de
ce contrat.

Une request possède au moins les dimensions concurrentes suivantes :

- les response headers peuvent arriver pendant l’écriture du request body ;
- une read et une write peuvent progresser indépendamment ;
- le request body se termine avec une collection éventuellement vide de request trailers ;
- le consumer contrôle le demand de lecture ;
- cancellation, timeout, peer reset et clean completion peuvent courir ;
- trailers et terminal outcome appartiennent à la fin du response stream ;
- release d’un handle et fin logique de l’opération ne sont pas nécessairement le même instant ;
- un shutdown de runtime doit englober toutes les notifications encore en vol ;
- un futur service host, comme un token provider, peut suspendre une request sans suspendre le
  runtime entier.

L’API publique voulue n’est pas une API gRPC. Le framing gRPC, `grpc-status`, retry et deadline
remote restent au-dessus. Le transport FFI porte HTTP/2 : headers, bytes, trailers et erreurs de
transport.

### 1.1 Contraintes de baseline

Le dépôt impose aujourd’hui deux contraintes qui orientent le choix :

- le client C# cible `netstandard2.0` et ses tests incluent .NET Framework 4.7/4.8 ;
- le client Java compile pour Java 17.

`LibraryImport` .NET moderne et le FFM API stable de Java 22 peuvent être des optimisations futures,
pas des prérequis. Le chemin de référence doit fonctionner avec `DllImport`/delegates ou JNI selon
le lowering retenu.

### 1.2 Objectifs

Le design doit :

- offrir une API fonctionnelle naturelle dans chaque langage ;
- rendre safety et liveness explicites, testables et composables par point de vue ;
- préserver le streaming duplex et son backpressure ;
- rendre impossible ou diagnostiquable la majorité des erreurs de binding ;
- permettre une implémentation performante sans copie native obligatoire des body chunks ;
- supporter les appels synchrones, les notifications et les host calls bidirectionnels ;
- mutualiser le runtime de binding pour de futures FFI ArmoniK ;
- permettre un shutdown déterministe avant destruction ou unload ;
- contenir une rupture d’invariant sans livrer de données au mauvais consumer ni prétendre à une
  recovery non démontrée ;
- versionner les capacités des deux côtés.

### 1.3 Non-objectifs

Le design ne cherche pas à :

- définir un runtime universel pour toutes les FFIs ;
- reproduire le Canonical ABI WebAssembly dans une DLL native ;
- garantir qu’un peer, un event loop arrêté ou du code utilisateur bloqué progresse ;
- fournir exactement-once au niveau réseau ;
- annuler de force du code foreign en cours d’exécution ;
- faire d’une DLL native in-process une security boundary contre du code natif malveillant ;
- rendre zero-copy toute projection vers un heap managé ;
- prouver la performance par le modèle formel.

## 2. Trois artefacts normatifs et deux niveaux de contrat

**Résumé.** Le contrat fonctionnel dit ce qu’est un transport ; le contrat d’isolation dit comment
deux runtimes s’échangent commands, events et ownership. Le premier doit rester stable quand le
second évolue. Le design relie les deux par des proof obligations. Les bindings sont des
implémentations de refinement entre les deux niveaux.

**Intention.** Le lecteur doit pouvoir choisir le document correspondant à sa question : comportement
fonctionnel, protocole d’isolation ou justification d’architecture. Aucun document ne doit obliger
à connaître l’implémentation Rust pour reconstruire un lifecycle ou une garantie.

La documentation normative cible doit finalement être séparée ainsi :

```text
TRANSPORT.md             normatif : contrat fonctionnel et lifecycles observables
PROTOCOL.md              normatif : ABI, isolation, modèle TLA+ et traces de conformance
DESIGN.md                normatif après décision : architecture et proof obligations
STATE_OF_THE_ART.md       informatif : références et limites connues
BASELINE_COMPARISON.md   informatif : constat sur les implémentations expérimentales
```

### 2.1 Contrat fonctionnel abstrait

Le lifecycle fonctionnel commence avant la request : le host construit une configuration, crée un
transport, établit une connection logique, puis choisit cette connection pour démarrer une call.
Une API language-neutral pourrait être décrite ainsi :

```text
Transport.create(TransportConfig) -> Transport
Transport.connect(Endpoint, ConnectionOptions) -> Future<Connection>
Connection.open(RequestHead) -> Call

Call.requestBody.write(bytes) -> Future<WriteAccepted>
Call.requestBody.complete(RequestTrailers) -> Future<Unit>
Call.responseHeaders -> Future<ResponseHead>
Call.responseBody -> AsyncByteStream
Call.completion -> Future<Result<ResponseTrailers, TransportError>>
Call.cancel(reason)

Connection.close() -> Future<Unit>
Transport.shutdown() -> Future<Unit>
```

`Connection` est une ressource logique contrôlant destination, security context et possibilité de
nouveaux starts. En HTTP/2, une connection physique multiplexe plusieurs calls concurrentes ; un
pool peut aussi sélectionner ou recréer plusieurs connections physiques derrière le même objet
logique. Le contrat doit donc préciser si l’appelant choisit un channel logique ou une socket
exacte. La recommandation est de ne pas promettre l’identité d’une socket : plusieurs requests
peuvent partager une session HTTP/2, et le pool choisit la session saine compatible. Si une future
API exige l’affinité à une session physique, ce sera une capability distincte.

La connection suit `New -> Connecting -> Open -> Draining -> Closed`. `close` refuse les nouveaux
starts puis attend ou annule les calls existantes selon la policy publiée. Le transport ne peut
être détruit qu’après la fermeture de ses connections ; une call garde la connection et le
transport vivants jusqu’à son terminal, même si le wrapper applicatif a demandé leur fermeture.

`WriteAccepted` signifie que le transport a pris en charge le buffer, pas que le peer l’a reçu.
`responseBody` est demand-driven. `completion` est terminal pour le transport ; il ne décide pas du
succès gRPC.

`RequestTrailers` est une collection éventuellement vide. `complete(empty)` produit un
`END_STREAM` sans trailing headers ; `complete(trailers)` envoie le dernier HEADERS frame puis ferme
le send side. Un overload idiomatique `complete()` peut appeler `complete(empty)`, mais le contrat
language-neutral ne doit pas supprimer une capacité HTTP/2 que le body channel sait représenter.
Les request trailers et les response trailers sont deux objets et deux lifecycles distincts.
La future de `complete` se résout lorsque le transport a accepté la frame terminale, pas lorsque le
peer l’a traitée. Une validation locale des trailer fields peut échouer avant l’envoi ; une course
avec cancel, reset ou terminal se résout par l’un des outcomes explicitement autorisés dans
`TRANSPORT.md`.

Au niveau fonctionnel, le caller ne modifie pas son buffer avant la completion de `write`, selon
l’idiome du langage. Au niveau ABI, le raw buffer host n’est borrowed que pendant le downcall : le
binding et Rust doivent avoir copié ou retained dans leur propre memory domain avant de retourner.
`WriteAccepted` décrit donc l’acceptation fonctionnelle du chunk, pas le droit de libérer un pointer
foreign. Les bytes de sortie appartiennent au transport jusqu’à leur transfert explicite ; la
projection les copie dans le heap host ou expose un lease dont `Dispose`/`close` appelle le release
natif. Aucun heap ne désalloue directement la mémoire d’un autre heap.

Cette interface peut être projetée en :

```csharp
Task<ResponseHead> ResponseHeaders { get; }
Stream ResponseBody { get; }
ValueTask WriteAsync(ReadOnlyMemory<byte> data, CancellationToken ct);
ValueTask CompleteRequestAsync(RequestTrailers trailers, CancellationToken ct);
Task<Completion> Completion { get; }
```

```java
CompletionStage<ResponseHead> responseHeaders();
Flow.Publisher<ByteBuffer> responseBody();
CompletionStage<Void> write(ByteBuffer data);
CompletionStage<Void> complete(RequestTrailers trailers);
CompletionStage<Completion> completion();
```

Dans les APIs idiomatiques, un overload sans argument peut déléguer à
`CompleteRequestAsync(RequestTrailers.Empty, ct)` ou `complete(RequestTrailers.empty())`. Le type
pluriel est intentionnel : HTTP autorise un bloc de plusieurs trailer fields, même si le cas vide
sera de loin le plus fréquent.

```python
headers = await call.response_headers
async for chunk in call.response_body:
    ...
completion = await call.completion
```

```cpp
// C++20 avec un type task fourni par la projection.
auto headers = co_await call.response_headers();
while (auto chunk = co_await call.read()) { ... }
```

Le wrapper C++11 peut offrir callbacks, `std::future` pour les résultats terminaux ou intégration à
un executor choisi. Le wrapper C++20 peut ajouter des awaiters. Aucun des deux ne modifie ce contrat
abstrait.

### 2.2 Contrat technique d’isolation

Le protocole technique doit être indépendant de HTTP dans son noyau :

```text
Runtime
Operation
Command
Event
OwnedPayload
Cancel
NotificationLowering
CallbackTrampoline | Wait/Wakeup
Release
Shutdown
Capability
```

Le transport ajoute ses operation kinds et event kinds : `RESPONSE_HEADERS`, `READ_DONE`,
`WRITE_DONE`, `COMPLETED`. Une future FFI de stockage ou d’administration pourra réutiliser la même
forme de runtime sans réutiliser l’API `Call`, mais seulement après que ce second cas aura démontré
quels éléments méritent d’être extraits du module transport.

### 2.3 Refinement attendu

Le binding doit raffiner les events techniques en observations fonctionnelles :

```text
RESPONSE_HEADERS(op, h)  -> complete(ResponseHeaders, h)
WRITE_DONE(op, n)        -> complete(pendingWrite, n)
CLOSE_SEND_DONE(op)      -> complete(pendingRequestTrailers)
READ_DONE(op, bytes)     -> yield(ResponseBody, bytes)
COMPLETED(op, ok, t)     -> end(ResponseBody); complete(Completion, t)
COMPLETED(op, error)     -> fail all pending functional operations
```

Ce mapping n’est pas purement syntaxique. Il décide l’ordre de publication entre end-of-stream et
trailers, la propagation de cancellation et le traitement des operations encore pending.

### 2.4 Catalogue minimal des lifecycles

Le tableau suivant fixe les questions que `TRANSPORT.md` et `PROTOCOL.md` devront répondre
normativement. « Destroy condition » signifie que l’owner a demandé la destruction et qu’aucun
borrow ou travail en vol ne peut encore accéder à l’objet.

| Objet | Allocateur et owner initial | Transfert/borrow | Destroy condition et désallocateur |
|---|---|---|---|
| `TransportConfig` host | host | sérialisée ou borrowed pendant `create` | retour de `create`; host/GC/C++ allocator |
| runtime Rust | Rust FFI sur `runtime_create` | handle opaque borrowed par les downcalls | `join` quiescent, aucun event/host call/thread ; Rust FFI |
| transport/client/pool | Rust transport | retained par connections et calls | fermeture + dernière référence ; Rust |
| connection logique | Rust transport | handle choisi par plusieurs calls | `Closed` + aucune call ; Rust |
| connection HTTP/2 physique | pool Rust | partagée par plusieurs streams | retirée du pool + aucun stream ; Rust/Hyper |
| call/operation | Rust FFI | registry Rust et token registry host | terminal + commands/events drainés + handles released ; chaque côté libère son état |
| input struct/bytes | host | borrowed uniquement pendant le downcall, sauf copy explicite | au retour ou après `WriteAccepted` selon l’API ; host |
| event batch storage | host ou binding | Rust écrit seulement dans la capacité validée | après traitement du batch ; même allocator host |
| callback event/payload C | Rust FFI | borrowed pendant le trampoline, ou retained explicitement avant retour | fin du callback ou release du retain ; Rust FFI |
| event/payload possédé W | Rust FFI | ownership transféré au binding par `wait` | exactement un `event_release`; Rust FFI |
| copie de payload host | binding/host heap | value idiomatique host | après dernier usage ; GC/RAII host |
| error buffer natif | Rust FFI | handle opaque lu par le binding | `error_release`; Rust FFI, même sur chemin d’échec |
| host-call invocation | Rust FFI + broker host | corrélée par ID, payload suivant les mêmes règles | reply/fail/cancel terminal + release ; chaque côté son état |
| callback context/root | binding | borrowed par Rust jusqu’à la barrière terminale | aucun callback en vol ; binding/GC/JNI |
| queue + dispatcher thread W | runtime propriétaire | références fortes des producers et du thread | gate fermé, producers finis, queue fermée/drainée, thread joint ; owner du runtime |

Cette table interdit deux raccourcis : un destructor host n’est pas une preuve de quiescence native,
et une fin logique n’est pas nécessairement la fin de lifetime. Les états de lifecycle seront des
variables du modèle, pas de simples commentaires autour des actions fonctionnelles.

## 3. Architecture logicielle factorisable

**Résumé.** La plomberie commune doit être concentrée dans un runtime d’isolation, puis réutilisée
par les adaptateurs de domaine. La factorisation entre langages porte sur le modèle, le schéma et
les tests ; la factorisation dans un langage porte aussi sur le code du dispatcher et des handles.

**Intention.** La première livraison doit factoriser à l’intérieur du domaine transport, là où les
invariants sont compris. Elle doit préparer une extraction future sans prétendre avoir déjà trouvé
un runtime FFI universel.

```text
Application / gRPC stack
          │ API fonctionnelle idiomatique
          ▼
Projection transport C# / Java / Python / C++
          │ futures, streams, erreurs
          ▼
Runtime de binding interne au package transport du langage
          │ operation registry, trampoline C, queue host, shutdown
          ▼
ABI C d’isolation
          │ commands, callbacks C, payloads
          ▼
`armonik_transport::runtime`
          │
          ▼
armonik-transport
```

### 3.1 Côté Rust

Dans un premier temps, un module `armonik_transport::runtime` fournit la machine fonctionnelle
indépendante de la C ABI :

- configuration, connection logique et calls concurrentes ;
- request/response headers, body demand et trailers ;
- streams et outcomes Rust typés ;
- cancellation et terminalization ;
- timeout, rate permit et flow control ;
- lifecycles fonctionnels et instrumentation indépendante du binding.

`armonik-transport-ffi` est la projection C mince de ce module. Elle possède l’isolation runtime,
les IDs generation-tagged, le trampoline callback, le callback result, les gates, la quiescence,
les capabilities et le failure containment. Le runtime transport n’importe aucune notion de
P/Invoke, JNI, GIL ou C callback.

Une extraction ultérieure vers
un crate `armonik-ffi-runtime` ne sera envisagée qu’après une seconde FFI démontrant quels concepts
ne dépendent réellement ni de HTTP/2 ni du transport.

### 3.2 Côté langage

Dans un premier temps, chaque langage inclut le runtime de binding dans son package fonctionnel :

```text
ArmoniK.Api.Client.Native.Internal       (.NET)
com.aneoconsulting.armonik.transport.internal (Java)
armonik.transport._native               (Python)
armonik::transport::detail              (C++)
```

Il contient : native loading, ABI check, safe handles, operation registry, trampoline C, queue host,
payload release, cancellation et shutdown. La frontière interne entre runtime de
binding et projection de domaine doit être nette et testée, même s’ils sont livrés dans le même
artefact.

Pour une seconde FFI fondée sur le même protocole, l’objectif raisonnable est de réutiliser 70 à
90 % de cette plomberie dans un langage. Ce chiffre est une cible d’architecture, pas une mesure
acquise. Une seconde FFI doit la valider avant extraction en package générique et stabilisation
publique du noyau.

### 3.3 Génération

Une source de schéma unique peut générer :

- C header et declarations par langage ;
- layouts, enum values et codecs ;
- capability IDs ;
- skeletons d’event routing ;
- conformance tests.

La state machine, les fairness assumptions et les règles d’ownership restent des entrées du
générateur ou des artefacts adjacents. Elles ne peuvent pas être inférées d’un prototype C.

### 3.4 Complexité vue par chaque utilisateur

« Utilisateur de la FFI » désigne plusieurs personas dont les coûts ne doivent pas être confondus :

| Persona | Surface manipulée | Complexité cible |
|---|---|---|
| application .NET/Java/Python/C++ | `Transport`, `Connection`, `Call`, future/stream, cancel/close | faible ; aucun token, event, root ou release natif |
| auteur de la projection transport | state machine request/response headers, body, trailers, erreurs et backpressure | moyenne ; code de domaine testé contre les traces fonctionnelles |
| mainteneur du runtime de binding | registry, dispatcher, owners, roots, shutdown, quarantaine | élevée une fois par langage ; code appartenant à la trusted computing base |
| auteur d’une nouvelle operation dans le même package | schéma, mapping command/event, tests de traces | faible à moyenne si elle réutilise les lifecycles existants |
| consumer direct de l’ABI C | totalité du protocole et de ses unsafe lifetimes | élevée ; usage avancé, non recommandé comme API fonctionnelle |

Les appels synchrones courts n’imposent pas de créer une operation ou de passer par le dispatcher.
Ils restent des méthodes ordinaires de la projection. À l’inverse, exposer directement le callback
ou la completion queue à l’application déplacerait la complexité trusted vers chaque consumer et
contredirait l’objectif d’isolation.

## 4. Alternative W non retenue en V1 : waitable runtime

**Résumé.** L’alternative W est une completion queue pull, batchable, avec events possédés. Elle
combine des downcalls synchrones courts et des operations asynchrones corrélées, sans reverse call
sur le chemin nominal. Elle n’est pas implémentée dans la V1 ; son modèle sert à rendre visibles les
coûts et assumptions que le callback retenu évite ou déplace.

**Intention.** Le message principal est de concentrer dans deux runtimes de confiance — Rust FFI et
binding — la queue, le routing, l’ownership, le backpressure et le shutdown. L’application ne doit
voir que `Transport`, `Connection`, `Call`, futures et streams idiomatiques.

### 4.1 Surface conceptuelle

La surface ci-dessous illustre le protocole, sans figer ses types binaires :

```c
ak_status ak_runtime_create(const ak_runtime_options *options,
                            ak_runtime **out_runtime,
                            ak_error *out_error);

ak_status ak_operation_start(ak_runtime *runtime,
                             const ak_operation_start_args *args,
                             ak_operation **out_operation,
                             ak_error *out_error);

ak_status ak_operation_command(ak_operation *operation,
                               uint32_t command_kind,
                               ak_bytes_in input,
                               ak_error *out_error);

ak_status ak_runtime_wait(ak_runtime *runtime,
                          ak_event *events,
                          size_t capacity,
                          ak_duration timeout,
                          size_t *out_count,
                          ak_error *out_error);

ak_status ak_event_release(ak_runtime *runtime,
                           ak_event event,
                           ak_error *out_error);
void ak_operation_cancel(ak_operation *operation);
void ak_operation_release(ak_operation *operation);

ak_status ak_runtime_request_shutdown(ak_runtime *runtime);
ak_status ak_runtime_finish_shutdown(ak_runtime *runtime,
                                     ak_duration timeout,
                                     ak_error *out_error);

ak_status ak_runtime_quarantine(ak_runtime *runtime,
                                uint32_t reason_code);
ak_status ak_runtime_join(ak_runtime *runtime,
                          ak_duration timeout,
                          ak_error *out_error);
```

Les appels de start et command empruntent leurs inputs seulement pendant le downcall. Un event
possède son payload jusqu’à `ak_event_release`. `runtime_wait` peut retourner plusieurs events pour
amortir les crossings. `runtime_quarantine` est non-blocking, idempotent et réveille les waiters ;
`runtime_join` est la barrière commune au shutdown normal et à la sortie sur faute. Ces signatures
restent conceptuelles : le chapitre 4.10 fixe leur sémantique avant leur forme binaire.

Les fonctions qui prennent plusieurs arguments n’utilisent ni varargs C ni une liste de pointers
dont les lifetimes diffèrent implicitement. Une operation complexe reçoit un input record versionné :

```c
typedef struct ak_operation_start_args {
    uint32_t size;                 /* sizeof de la version connue du consumer */
    uint16_t version;
    uint16_t flags;
    ak_connection *connection;    /* connection logique choisie */
    ak_token consumer_token;
    uint32_t operation_kind;
    uint32_t reserved;
    ak_bytes_in request_head;
    ak_bytes_in initial_body;
    ak_duration deadline;
} ak_operation_start_args;
```

Les scalars réellement stables peuvent rester des paramètres directs. Les records évolutifs portent
`size`, `version`, `flags` et des champs reserved à zéro. Une array est toujours `ptr + len`; un
optional est indiqué par un flag ou une représentation nulle définie. Rust valide `size`, version,
alignment, overflow, flags, nullability et capabilities avant toute lecture. Il copie tout champ
dont il a besoin après le retour. Les nested pointers sont donc borrowed pendant le downcall et ne
deviennent jamais implicitement owned.

Les resources de domaine ont des fonctions explicites, même si leur lowering interne réutilise
`operation_start` : `transport_create(config)`, `connection_open(transport, options)` puis
`call_start(connection, request)`. Cela rend impossible le démarrage d’une call sans choisir la
connection logique. Un connect asynchrone publie son propre terminal event ; la création du runtime
et la validation locale de configuration restent synchrones et bornées.

### 4.2 Correlation token choisi par le consumer

Le binding enregistre son état avant de permettre une completion rapide :

```text
token = registry.allocate(callState)
status, nativeHandle = operation_start(runtime, token, request)
if status != OK:
    registry.remove(token)
    failSynchronously(status)
```

Une operation ayant démarré peut publier un event avant le retour de `operation_start`; cet event
reste dans la queue et son token est déjà connu. Si `start` échoue synchroniquement, aucun event ne
sera publié pour ce token. Cette règle doit être normative et testée.

Le token n’est ni une adresse ni une capability d’autorisation. Il doit contenir une identité de
runtime ou une generation et ne pas être réutilisé pendant la vie du runtime. Le binding refuse une
collision avant le downcall ; le runtime refuse atomiquement un token déjà actif. Une largeur de
128 bits simplifie la séparation `runtime_epoch + counter`, mais le choix exact reste à mesurer. Un
counter 64 bits est acceptable seulement si le non-wrap est un invariant construit, pas une
hypothèse probabiliste.

Le `nativeHandle` conceptuel doit suivre la même règle. Une adresse de heap utilisée comme unique
identité ne distingue pas un objet libéré d’un nouvel objet réalloué à la même adresse. Le format
figé devra être un slot avec generation, un integer opaque non réutilisé, ou une indirection offrant
une propriété équivalente. Le runtime peut utiliser une représentation pointer-shaped pour l’ABI,
mais il ne doit jamais la déréférencer avant validation dans sa registry.

### 4.3 Event possédé

Un event conceptuel contient :

```text
consumer_token
event_id
operation_kind
event_kind
status
flags
payload { ptr, len, owner }
```

Le binding doit release même un event inconnu. Un event additif inconnu n’est ignorable en sécurité
que si :

- son payload peut être libéré génériquement ;
- il ne demande ni acknowledgement spécifique ni nouvel armement ;
- il ne modifie pas l’ownership d’une ressource existante ;
- il n’introduit pas de nouvelle terminal semantics.

Sinon il exige une capability négociée avant start.

`event_id` et l’owner du payload doivent être opaques. `event_release` valide le runtime, la
generation et l’état `Transferred` avant de libérer ; il ne doit pas déréférencer un pointer fourni
comme preuve d’ownership. Un double release ou un event appartenant à un autre runtime retourne une
rupture de protocole et déclenche la quarantaine. Cette validation a un coût de registry ; les
benchmarks compareront release unitaire, batch release et handles indexés avec generation.

### 4.4 Backpressure

Trois backpressures doivent rester distincts :

1. HTTP/2 flow control entre transport et peer ;
2. demand du response body entre projection et application ;
3. capacité de la event queue entre Rust et binding.

Une read command produit au plus une `READ_DONE` ou est résolue par `COMPLETED`. Le runtime ne doit
pas lire indéfiniment le body si aucune read n’est armée, sauf pendant un drain terminal explicitement
défini. La queue d’events fonctionnels ne doit jamais drop silencieusement. Une queue de diagnostics
peut avoir une politique de drop distincte.

Une `CLOSE_SEND` command porte un blob de `RequestTrailers`, éventuellement vide. Elle est exclusive
d’une write pending, ferme définitivement le send side et se résout par `CLOSE_SEND_DONE` ou par le
terminal de la call. Le binding conserve le waiter et les bytes nécessaires jusqu’à l’une de ces
issues ; Rust copie le blob avant le retour du downcall.

Une queue bornée peut bloquer la publication d’un event ; le task Rust concerné doit alors await de
la capacité sans bloquer son worker. Le modèle TLA+ doit inclure `queueCapacity` et le consumer
drain, faute de quoi la preuve de liveness suppose implicitement une queue infinie.

### 4.5 Appels synchrones

Tous les appels ne doivent pas passer par la queue. La classification proposée est :

| Classe | ABI | Exemples |
|---|---|---|
| Court, local, borné | downcall direct | ABI version, capability query, config validation |
| Déclenchement sans attente | downcall direct | cancel request, request shutdown |
| I/O ou attente potentiellement non bornée | operation async | request, DNS, certificate provider |
| Barrière | async ou wait explicite | finish shutdown, drain |

Règle normative cible :

> Un appel synchrone ne dépend pas du progress du dispatcher et termine dans un temps borné, hors
> contention locale explicitement documentée.

Une fonction de configuration concurrente doit également préciser si elle affecte les operations
déjà démarrées. Par défaut, une mutation s’applique seulement aux starts futurs.

### 4.6 API bidirectionnelle

Les notifications sans réponse sont des events runtime. Les host calls qui attendent une réponse
sont corrélés :

```c
ak_status ak_host_call_complete(ak_runtime *runtime,
                                uint64_t invocation_id,
                                ak_bytes_in result);

ak_status ak_host_call_fail(ak_runtime *runtime,
                            uint64_t invocation_id,
                            ak_status error,
                            ak_bytes_in message);
```

Exemple logger :

```text
LOG_RECORD -> dispatcher -> ILogger.Log
```

Le logger ne participe pas à la liveness fonctionnelle. `DEBUG` et `TRACE` peuvent être droppés
selon une politique publiée ; une erreur fonctionnelle doit aussi exister comme résultat structuré.

Exemple token provider :

```text
HOST_CALL(id, GetToken, request)
HOST_CALL_REPLY(id, token | error)
```

La request native est suspendue, pas le dispatcher. Le host call doit finir par `reply`, `fail` ou
`cancelled`. Le shutdown résout les invocations restantes.

### 4.7 Shutdown

Le shutdown est un protocole et non un destructor :

```text
Running
  -> request_shutdown
Stopping        aucun nouveau start accepté
  -> cancel/finish operations
Draining        events et host calls terminaux encore retirés
  -> SHUTDOWN_COMPLETE observé
Stopped         aucun nouvel event, handles libérables, DLL unloadable
```

`SHUTDOWN_COMPLETE` est une barrière : après son observation et la release des events précédents,
le binding sait qu’aucun native thread ne touchera plus son registry ou ses services host.

### 4.8 Ownership, lifecycles, safety et liveness de l’alternative

La règle d’allocation est locale à chaque memory domain :

> Toute mémoire est désallouée par l’allocator et le langage qui l’ont allouée, seulement après la
> fin de tous les borrows et accès en vol. Traverser l’ABI transfère une valeur, un borrow borné ou
> un opaque owner ; cela ne transfère jamais implicitement le droit d’appeler `free` ou `delete`.

Concrètement :

- un input host est accessible par Rust uniquement pendant le downcall ; Rust le copie ou le retain
  par un mécanisme explicitement négocié avant de retourner ;
- un event Rust reste valide jusqu’à `event_release`; cet appel invalide immédiatement tous ses
  `ptr`, et seul Rust libère son allocation ;
- si le binding copie le payload, la copie est un nouvel objet host libéré par GC, RAII ou
  l’allocator host ; le release de l’event natif reste indépendant ;
- un wrapper C++11/C++20 peut appeler `delete` sur ses propres objets `new`, mais appelle les
  fonctions `*_release` pour les handles natifs ; il n’applique jamais `delete` à un pointer Rust ;
- une root .NET/JNI/Python est libérée par le binding seulement après la barrière prouvant qu’aucun
  callback, event routable ou continuation interne ne peut encore la référencer ;
- un output partiellement construit n’est jamais rendu owned sur erreur : soit l’appel réussit et
  renseigne l’owner, soit il échoue et seul `ak_error` possède une ressource releasable.

Un `runtime`, une `connection`, une `operation`, une queue, un dispatcher thread, un event et un
host call ont donc chacun un état de lifecycle explicite. Leurs destroy actions sont enabled
seulement lorsque leurs children sont terminés et leurs in-flight counters nuls. Cette relation est
modélisée dans `CallLifecycle.tla` et `MemoryOwnership.tla`, puis testée par des variantes où
`close`, cancellation, completion et release courent.

Safety de l’alternative :

- S1 : un token n’est routé qu’à une operation enregistrée ;
- S2 : chaque event publié est release exactement une fois ;
- S3 : `COMPLETED` apparaît au plus une fois et est terminal pour son operation ;
- S4 : un command acknowledgement résout au plus une command pending ;
- S5 : aucune operation n’est démarrée après `Stopping` ;
- S6 : aucun event n’est publié après `SHUTDOWN_COMPLETE` ;
- S7 : chaque host-call reply vise une invocation pending ;
- S8 : cancellation peut perdre contre completion, mais ne crée pas deux terminal outcomes ;
- S9 : un event visant un token inconnu, stale ou déjà terminal n’est jamais livré ;
- S10 : la première rupture de protocole ferme atomiquement le start gate du binding et latch la
  première cause avant toute nouvelle livraison applicative ;
- S11 : `runtime_quarantine` ferme atomiquement le start gate natif ; un start concurrent déjà passé
  côté host échoue ou devient une operation connue immédiatement terminalisée, jamais une operation
  qui échappe à la quarantaine ;
- S12 : chaque operation connue est résolue au plus une fois, y compris pendant la quarantaine ;
- S13 : un runtime n’est détruit ou unload qu’après une barrière de quiescence réussie ;
- S14 : un input réseau invalide ne peut produire que des identifiants déjà alloués par le runtime ;
- S15 : diagnostics et tombstones restent bornés indépendamment du nombre de fautes.

Les garanties de liveness suivantes sont conditionnelles. Les clauses introduites par « si » sont
des assumptions sur l’environnement du composant qui fournit la garantie, non des garanties du
composant lui-même :

- L1 : une operation commencée finit si le peer finit, si les host calls requis répondent, si le
  consumer maintient le demand nécessaire et si les schedulers sont fair ;
- L2 : une read armée se résout si le peer produit/end le stream et si la queue est drainée ;
- L3 : un shutdown finit si l’application continue à drainer et si aucun service host/callback
  externe ne reste bloqué ;
- L4 : un event en queue est finalement routé sous weak fairness du dispatcher ;
- L5 : une cancellation finit par être observée sous weak fairness de la request task, sans promesse
  de rollback distant ;
- L6 : si le dispatcher continue un step après détection, le runtime atteint `Quarantining` sans
  attendre le peer, l’application ou la capacité de la queue fonctionnelle ;
- L7 : sous fairness des native tasks et retour des host calls déjà engagés, `Quarantining` atteint
  `FailedQuiescent`; sans cette hypothèse, une deadline locale conduit à `FailedUnquiesced` plutôt
  qu’à une attente non bornée présentée comme un shutdown.

### 4.9 Contrats locaux de l’alternative waitable

Cette table ne cherche pas un « owner » unique pour une propriété composée. Elle énonce le contrat
de chaque couche ; les propriétés end-to-end seront des théorèmes de composition au chapitre 9.

| Couche observée | Garanties propres | Assumptions reçues | Décharge attendue |
|---|---|---|---|
| Rust fonctionnel | peer/timeout/cancel → outcome interne ; input réseau confiné | task et timer schedulés ; host call résolu | runtime Rust/OS ; chaîne host-call |
| Rust FFI | command acceptée → event ou terminal ; queue sans perte ; owner atomique ; barrière native fidèle | outcome Rust produit ; capacité finalement disponible ou shutdown | Rust fonctionnel ; runtime de binding |
| runtime de binding | event retiré → route ou quarantaine puis release ; dispatcher et roots drainés | event conforme ; dispatcher schedulé ; barrière native reçue | Rust FFI ; runtime host |
| host fonctionnel | demand/cancel/request trailers → commands ; observations idiomatiques exactement une fois | binding fidèle ; consumer demande, annule ou ferme | runtime de binding ; application |

La liveness globale n’appartient à aucune ligne isolée. Elle résulte de leur composition sous les
assumptions résiduelles explicites : CPU, timer local et retour du code utilisateur.

### 4.10 Protocole de sortie sur rupture d’invariant

**Résumé.** Une rupture d’invariant de correlation ou d’ownership met en quarantaine le runtime qui
l’a produite. Le binding ne livre plus d’event applicatif, le runtime refuse les nouveaux starts,
les operations connues échouent avec une cause commune, puis une barrière distingue une instance
quiescent et destructible d’une instance encore dangereuse à toucher. Cette réaction est stricte,
mais sa portée doit rester le runtime : un peer ne doit pas pouvoir provoquer cette transition avec
un simple message réseau invalide.

**Intention.** Le lecteur doit retenir qu’un invariant cassé ferme le chemin applicatif avant toute
recovery. La priorité est de ne pas router ou libérer sur une preuve d’ownership douteuse ; la
réutilisation de l’instance fautive n’est jamais implicite.

#### 4.10.1 Threat model et actifs protégés

Le protocole protège quatre actifs : l’integrity du routing et de la mémoire, la confidentiality des
payloads et credentials, l’availability du host et la fiabilité des diagnostics. Il traite les
fautes accidentelles du runtime ou du binding et les inputs contrôlés par une application ou un peer
malveillant. Il apporte une défense en profondeur contre un binding incorrect.

Il ne protège pas le process contre une DLL native malveillante ou un undefined behavior ayant déjà
corrompu l’address space. Une bibliothèque native chargée in-process partage la mémoire et les
privilèges du host. Si ce composant doit être non fiable, le design doit passer à un worker process
ou à un sandbox ; aucun `operation_id`, checksum ou `catch_unwind` ne transforme l’ABI C en security
boundary.

Le peer réseau ne choisit jamais `consumer_token`, `event_id`, native handle ou `invocation_id`.
Ces valeurs sont créées à l’intérieur du protocole d’isolation. Un parser HTTP/2 ou un service
distant qui pourrait injecter directement l’un de ces identifiants violerait le threat model et
constituerait un défaut de design.

#### 4.10.2 Classification avant réaction

La portée de la faute est déterminée avant toute tentative de recovery :

| Observation | Scope | Réaction | Runtime réutilisable ? |
|---|---|---|---|
| argument invalide, taille hors limite, duplicate token refusé avant mutation | downcall | status synchrone, aucune transition globale | oui |
| frame réseau malformée, peer reset, payload fonctionnel invalide | operation ou connection | terminal error, éventuellement fermeture de connection | oui |
| event kind additif explicitement ignorable et génériquement releasable | event | release, metric bornée, continuation | oui |
| result code inconnu dans un terminal event bien formé | operation | failure générique, release ; incident de compatibility | oui, selon capability policy |
| `operation_id` inconnu, stale ou déjà terminal | runtime | quarantaine immédiate | jamais la même instance |
| duplicate terminal, command ack sans command, host reply native sans invocation | runtime | quarantaine immédiate | jamais la même instance |
| double release, wrong-runtime owner, generation mismatch | runtime | quarantaine immédiate | jamais la même instance |
| longueur impossible, owner illisible, ABI layout incompatible, corruption suspectée | process/isolation unit | ne pas déréférencer ni tenter un cleanup complexe ; politique fail-fast ou kill du worker | non |
| panic d’une task confinée avant publication d’état partagé | operation | `INTERNAL_PANIC` terminal et incident | oui si ce confinement est démontré |
| panic du registry, dispatcher ou shutdown controller | runtime | quarantaine | jamais la même instance |

« Event inconnu » et « operation inconnue » ne sont donc pas symétriques. Le premier peut être une
extension préparée par le contrat. Le second rend impossible de savoir à quel état managé et à
quelle lifetime rattacher le payload. L’ignorer pourrait strand une task, fuiter un owner ou masquer
un event envoyé après free.

Cette classification évite aussi un denial of service trivial. Une frame distante invalide est un
input attendu d’un parser hostile et ferme son scope fonctionnel. Elle ne doit devenir un
`ProtocolFault` global que si elle révèle ensuite une contradiction interne indépendante de la
valeur distante.

#### 4.10.3 State machine de quarantaine

Le shutdown normal et la rupture d’invariant partagent une barrière, mais pas leur signification :

```text
Running
  | DetectProtocolFault(firstCause)
  v
Quarantining        start/command/host-call nouveaux refusés
  | native abort demandé, waiters réveillés, operations connues détachées
  v
FailedDraining      aucun event fonctionnel livré ; owners transférés libérés
  | join réussit                         | deadline de join
  v                                      v
FailedQuiescent                         FailedUnquiesced
  | destruction autorisée                | conserver mémoire et roots
  | nouveau runtime explicite             | kill worker ou fail-fast host
  v                                      v
Destroyed                                terminal
```

Deux linearization points sont nécessaires. `DetectProtocolFault` ferme d’abord atomiquement le
start gate du binding et conserve la première cause avec un `fault_epoch` immuable ; les
observations suivantes incrémentent seulement des counters bornés. `runtime_quarantine` ferme
ensuite atomiquement le gate natif. Un downcall déjà engagé entre les deux points peut être accepté
par Rust, mais son operation est alors connue et terminalisée par la quarantaine. La transition ne
dépend ni de la queue fonctionnelle, ni du logger applicatif, ni du peer. `runtime_quarantine` doit
donc être un downcall court, idempotent et sans allocation non bornée.

`FailedUnquiesced` est un état terminal exploitable, pas un échec de documentation. Dans cet état le
binding ne free pas les roots, ne détruit pas les registries, ne unload pas la DLL et ne réutilise
pas les handles. Trois politiques host sont possibles : conserver intentionnellement l’instance
jusqu’à la fin du process, terminer le process avec un mécanisme fail-fast, ou tuer et recréer un
worker isolé. La bibliothèque ne choisit pas silencieusement entre availability et process
termination.

#### 4.10.4 Exemple : réception d’un `operation_id` inconnu

Le dispatcher applique l’ordre suivant :

```text
batch = runtime_wait()
for index, event in batch:
    validateEnvelopeBounds(event)       // aucune lecture du payload avant ce check
    state = registry.lookup(event.consumer_token)
    if state is Missing | Tombstone:
        fault = latch(UNKNOWN_OPERATION, metadataWithoutPayload(event))
        closeBindingStartGate(fault)
        runtime_quarantine(fault.code)
        releaseCurrentAndRemainingWellFormedEvents(batch, index)
        detachAndFailAllKnownOperations(fault)
        outcome = runtime_join(quarantineDeadline)
        applyHostPolicy(outcome)
        stopDispatcher()
    else:
        routeThenReleaseExactlyOnce(state, event)
```

Le registry conserve des tombstones bornés ou, de préférence, une generation non réutilisée afin de
distinguer un late event d’un token jamais émis. Les deux sont des violations ; la distinction sert
au diagnostic, pas à continuer. Les tombstones ont une capacité fixe et une eviction qui ne rend
jamais un token réutilisable dans le même runtime.

Les events précédemment traités ont déjà été released et ne doivent pas l’être une seconde fois. Le
current event et les events restants d’un batch déjà transféré au binding doivent être libérés
génériquement sans invoquer l’application. Si
l’envelope ou son owner est lui-même invalide, le binding ne tente ni `Marshal.Copy`, ni
déréférencement, ni release fondé sur ce champ. Il latch `CORRUPT_EVENT`, conserve le runtime et
escalade vers la politique d’isolation. Cette fuite bornée est préférable à un cleanup sur une
adresse non fiable.

Toutes les operations connues sont retirées atomiquement du routing normal puis terminées une fois
avec `ProtocolViolationException` ou son équivalent. Les continuations sont postées sur le scheduler
du binding ; elles ne sont pas exécutées inline dans le dispatcher en faute. Aucun retry fonctionnel
n’est automatique : une write distante peut avoir eu un effet avant la rupture et seule la couche
fonctionnelle connaît l’idempotency nécessaire à un retry.

#### 4.10.5 Diagnostics et résistance au denial of service

Le fault record est structuré et borné. Il contient au plus : reason code stable, ABI et capability
versions, runtime instance ID, build ID, direction, event kind, sequence/generation, état du runtime
et counters. Il ne contient pas le payload, les headers, credentials, raw pointers ou message
d’erreur distant. Un identifiant nécessaire à la corrélation est pseudonymisé si les logs changent
de trust zone.

Les strings étrangères sont length-limited, normalisées et encodées comme data structurée ; CR, LF
et delimiters ne doivent pas créer de faux records. Seule la première faute produit le diagnostic
complet. Les suivantes sont agrégées, afin qu’un incident ne remplisse pas mémoire, disque ou queue
de telemetry. Un crash dump, potentiellement riche en secrets, relève d’une politique explicite du
host.

La recovery est elle aussi bornée : quota de runtimes quarantined, circuit breaker et exponential
backoff empêchent une boucle `fault -> recreate` d’amplifier une attaque. Lorsque plusieurs tenants
ou clients ne se font pas confiance, un runtime process-wide augmente le blast radius ; plusieurs
instances ou worker processes fournissent un meilleur compartmentalization au prix de threads,
connections et mémoire supplémentaires.

#### 4.10.6 Contrats locaux de confinement

| Couche observée | Garanties propres | Assumptions reçues | Issue si l’assumption manque |
|---|---|---|---|
| Rust fonctionnel | input réseau invalide → erreur operation/connection ; aucun isolation ID issu du peer | parser obtient du CPU ; mémoire interne intacte | timeout local ou failure de l’isolation unit |
| Rust FFI | valide handle/generation/owner avant usage ; ferme le gate natif ; réveille les waiters ; publie la quiescence native | downcall de quarantaine exécuté ; tasks natives schedulées | `FailedUnquiesced` côté natif |
| runtime de binding | valide envelope/token avant payload ; ferme d’abord le gate host ; stoppe les livraisons ; fail once les states connus | dispatcher exécute son step ; scheduler accepte les completions | gate fermé, roots retained, alerte |
| host fonctionnel | applique retain, worker kill ou fail-fast ; aucun retry non idempotent implicite | OS exécute la policy | process déjà hors du modèle |

`FailedQuiescent` end-to-end est composé de la quiescence native **et** de la quiescence du binding ;
aucune des deux couches ne peut la garantir seule. Le protocole doit donc distinguer la barrière
native d’un éventuel `BindingQuiescent` local avant destruction totale.

Les politiques de diagnostics, quota, alerting et retry/idempotency ne déchargent pas, par elles-mêmes,
une assumption de fairness. Elles restent des décisions host et fonctionnelles explicites, testées séparément. Si une
assumption de la table n’est déchargée par aucune garantie voisine, le contrat promet
`FailedUnquiesced` et une alerte, pas `FailedQuiescent`.

## 5. Design V1 retenu : callback MsQuic-style vers un trampoline de binding

**Résumé.** Le callback direct est le design V1. Sa target est exclusivement un trampoline du
runtime de binding. Le trampoline publie l’observation dans l’état host, ne lance
aucun user code, contient toute exception et retourne. Il doit être évalué comme un protocole
complet comparable à MsQuic, pas comme un function pointer applicatif ajouté à une task Tokio.

**Intention.** Chaque host binding doit fournir `TrampolineReturns` comme garantie locale. Rust ne
dépend alors plus de la terminaison du code utilisateur. Un binding qui ne sait pas établir cette
garantie n’est pas conforme ; il doit proposer et faire revoir un lowering d’isolation distinct.

### 5.1 Forme correcte minimale

Le callback visible par Rust doit être un trampoline de binding, pas un logger ou handler utilisateur
arbitraire :

```text
Rust callback -> trampoline binding -> transition locale ou queue interne -> retour
                                      -> executor -> continuation utilisateur
```

Les règles minimales sont :

- état publié avant que le callback soit possible ;
- events serialized par operation ;
- flags d’armement libérés avant callback si rearming reentrant est permis ;
- aucune lock Rust conservée pendant l’upcall ;
- liste explicite des reentrant downcalls autorisés ;
- aucun reentrant downcall autorisé à attendre un event futur de la même operation ;
- exception/unwind containment des deux côtés ;
- payload valide pour la durée du callback ou représenté par un owned handle ;
- in-flight callback count et deferred destruction ;
- shutdown barrier garantissant zéro callback tardif.

Pour la V1, la liste des reentrant downcalls ordinaires est vide : le callback
retourne sa disposition, puis demand/rearm/cancel sont postés et exécutés après son retour. Cela
réduit fortement la preuve d’absence de lock cycle. La baseline sait libérer ses flags avant
l’upcall et peut donc autoriser le rearm ; l’adapter V1 remplace ce rearm inline par une command
postée après retour. Si un benchmark futur justifie un rearm inline, il devient une capability
séparée et chaque downcall ajouté doit avoir une precondition et une preuve de non-attente sur la
même operation.

### 5.2 Invocation directe du trampoline fini

Le chemin minimal appelle directement le trampoline depuis la request task. Il n’utilise ni
`spawn_blocking` ni dispatcher natif :

```text
Rust task -> trampoline officiel -> validate/copy-or-retain/complete-local-state -> return
                                                                    |
                                                                    v plus tard
                                                               host executor -> user code
```

Le nombre d’observations fonctionnelles non consommées est borné **par operation** : une read et une
write au plus sont armées, headers et terminal n’arrivent qu’une fois. Le binding prépublie les
slots correspondantes avant start/armement. Le bound global exige donc aussi un quota explicite de
calls actives ; sans ce quota, l’argument de boundedness serait faux. Le chemin C n’a pas besoin
d’une queue native fonctionnelle générique, mais le runtime host peut encore posséder une queue de
scheduling : elle doit réutiliser les slots par operation, refuser explicitement la saturation ou
être bornée par le même quota. Les diagnostics restent sur une voie lossy séparée.

Le callback peut retourner une disposition conceptuelle :

```c
ak_callback_result on_event(void *runtime_ctx, const ak_event_borrowed *event);
/* ACCEPTED | OPERATION_FAULT | RUNTIME_FAULT */
```

`runtime_ctx` désigne la registry du runtime de binding, rooted jusqu’à la barrière globale ; ce
n’est pas un pointer vers l’objet d’une operation. L’event porte le `consumer_token` préenregistré.
Le trampoline peut ainsi traiter un token inconnu comme une rupture de protocole sans tenter de
déréférencer un contexte d’operation stale.

`ACCEPTED` signifie que le binding a effectué sa transition et copié ou retained le payload.
`OPERATION_FAULT` termine la seule operation lorsque l’intégrité partagée reste démontrée ;
`RUNTIME_FAULT` ferme le gate et déclenche la quarantaine. Aucun résultat ne demande à Rust
d’attendre une continuation host. L’allocation impossible, l’exception interceptée et le token
inconnu ont ainsi une issue observable au lieu d’être avalés.

La garantie `TrampolineReturns` doit être établie sur un code très petit : aucune lock susceptible
d’attendre Rust, aucun appel réseau, aucun logger applicatif, aucun `await`, aucune continuation
inline et aucun downcall ordinaire. La fast-completion race est fermée par le token/state publié
avant start. Un GC stop-the-world ou un runtime host suspendu peut encore retarder le retour : c’est
une limite in-process à mesurer, pas une raison d’ajouter automatiquement `spawn_blocking`.

### 5.3 `spawn_blocking(callback).await`

Cette variante protège les workers Tokio et préserve naturellement l’ordre par request. Elle ajoute
l’hypothèse :

```text
CallbackStarted => <>CallbackReturned
```

La request task ne progresse pas avant le retour. Un callback qui effectue un sync-over-async sur
une read dépendant de cette task deadlock. Une blocking task commencée n’est pas abortable ; le
shutdown peut donc attendre indéfiniment ou abandonner une task toujours capable de rappeler le
host.

Elle impose aussi un payload `Send + 'static` à la closure. Un raw pointer emprunté à une frame ne
peut pas être capturé en sécurité ; il faut transférer un `Bytes`/`Arc` ou copier.

### 5.4 `spawn_blocking(callback)` détaché

Le transport ne dépend plus du retour immédiat, mais il faut recréer :

- une queue par operation ou un serial executor ;
- une barrière de completion après tous les callbacks précédents ;
- un compteur global pour le shutdown ;
- un ownership de payload jusqu’au retour réel ;
- un bound et une politique de saturation.

Sans ces mécanismes, `COMPLETED` peut être observé avant un `READ_DONE` déjà programmé ou après la
libération de `ctx`. Avec eux, la variante ressemble à une completion queue push distribuée sur le
pool blocking global.

### 5.5 Dispatcher thread natif dédié

Une variante plus maîtrisable utilise une queue Rust et un thread d’upcall dédié par runtime :

```text
Tokio tasks -> owned event queue -> callback thread -> binding trampoline
```

Elle conserve un modèle push, l’ordre et un point de shutdown. Java peut attacher ce thread JNI une
fois. Elle ajoute cependant un thread et garde toutes les contraintes de reverse FFI. C’est le
comparateur d’isolation callback à mesurer seulement si les pauses du runtime host rendent le
trampoline direct incompatible avec les workers Tokio.

La queue et le thread forment une seule unité de lifecycle. Le runtime les alloue avant d’accepter
le premier start. Le thread détient une strong reference sur la queue et le runtime ; chaque
producer détient une référence qui disparaît après sa dernière publication. Le shutdown suit cet
ordre : fermer le enqueue gate, terminer ou annuler les producers, fermer la queue, réveiller le
thread même si elle est vide, drainer les events autorisés, publier la barrière terminale, joindre
le thread, puis seulement libérer queue, callback root et runtime. Un timeout de join ne permet
jamais de free la queue sous un thread encore bloqué : l’unité reste retained en
`FailedUnquiesced` ou son worker process est tué.

### 5.6 Contrats locaux des callbacks directs

| Couche observée | Garanties propres | Assumptions reçues | Décharge attendue |
|---|---|---|---|
| Rust FFI | ordering par operation ; aucun lock exposé ; downcalls reentrant explicitement listés ; zéro nouvel upcall après barrière native | trampoline retourne ; callback state reste live | runtime de binding officiel |
| trampoline du binding | state/root publiés avant start ; validation/copie/retain ; transition atomique ; aucune continuation utilisateur inline ; exception contenue ; retour garanti sous ses assumptions | event conforme ; runtime host utilisable ; thread obtient du CPU ; allocation retourne succès/échec | Rust FFI ; runtime du langage et OS |
| host fonctionnel | continuation postée sur son executor ; demand/cancel/close deviennent des downcalls ultérieurs | trampoline a publié l’observation ; executor accepte le travail | runtime de binding ; runtime host |

Cette forme change la conclusion sur `CallbackReturns`. Rust FFI la reçoit toujours comme
assumption, mais l’application n’en est plus le fournisseur : le runtime de binding, inclus dans la
trusted computing base, la garantit. Le reactor Rust n’attend jamais un handler applicatif ; il
attend seulement un trampoline fini et auditable. Le retour du handler reste une assumption
résiduelle uniquement pour les propriétés applicatives qui l’attendent ; l’application n’est pas
une couche du chemin de callback natif.

### 5.7 Appels synchrones et API bidirectionnelle

Les appels synchrones courts restent des downcalls directs. Plusieurs arguments traversent la
frontière dans une args struct versionnée par `size/version`, ou comme values/scalars explicites ;
toute string ou slice est borrowed pendant l’appel et copiée si Rust doit la conserver. Un appel
classé synchrone ne déclenche aucun callback avant son retour et n’attend ni peer ni host scheduler.

Un logger fourni par C# à Rust est une notification unidirectionnelle qui emprunte le même
trampoline, mais pas le code utilisateur : Rust publie un `LOG_RECORD`, le trampoline copie le
record dans une voie host bornée et retourne ; le worker appelle `ILogger.Log` plus tard. Les niveaux
diagnostics peuvent être droppés avec compteur. Une faute fonctionnelle ou de sécurité ne dépend
jamais de la disponibilité du logger.

Un host service dont Rust attend une valeur, tel un token provider, utilise request/reply :

```text
Rust task -> HOST_CALL(invocation_token, arguments) -> trampoline -> host queue -> retour
host worker -> service utilisateur -> host_call_reply/fail/cancel(invocation_token) -> Rust task
```

Le reply est un downcall ultérieur, jamais reentrant depuis le trampoline. L’invocation possède un
timeout/cancel path et reste child du runtime jusqu’à reply, fail ou cancel. Le shutdown ferme
d’abord le start gate, refuse les nouvelles invocations, résout celles en vol selon la policy, puis
attend leur quiescence. Le modèle `HostCalls.tla` est transverse aux quatre points de vue.

### 5.8 Triggers de réouverture

Le choix callback doit être rouvert si son adoption oblige chaque binding officiel à réimplémenter
indépendamment attach/detach, rooting, serialization et shutdown sans noyau commun testable, si une
cible ne peut pas garantir le retour borné, ou si les mesures montrent un impact incompatible avec
les SLOs. La décision suivante compare alors dispatcher natif, mécanisme propre à la cible et
waitable ; elle n’introduit pas implicitement deux protocoles dans tous les bindings.

## 6. Candidat F : future handles par opération élémentaire

**Résumé.** Les future handles sont une bonne primitive pour start, read et write pris isolément.
Ils ne suppriment pas le besoin d’un dispatcher de wake ni de la machine de transport. Ils peuvent
être un lowering interne de l’alternative waitable, mais ne sont pas retenus comme architecture complète
duplex sans prototype supplémentaire.

**Intention.** Le lecteur doit retenir qu’une collection de futures exactement-once ne constitue
pas une preuve de liveness d’un stream duplex. Il faut encore owner la machine qui coordonne
ordering, demand, cancellation et destruction des handles.

Exemple :

```text
request_start -> RequestHandle
request_headers_future(request) -> FutureHandle
request_write(request, bytes) -> FutureHandle
request_read(request) -> FutureHandle
request_completion_future(request) -> FutureHandle
```

Le binding doit gérer plusieurs handles et leurs races de cancellation/free. Un callback de wake
par future réintroduit reverse FFI ; un waitable set commun revient à la completion queue. Le modèle
est surtout intéressant si le générateur sait produire des futures simples pour de nombreuses APIs
non streaming.

| Couche observée | Garanties propres | Assumptions reçues |
|---|---|---|
| future native | readiness monotone ; wake conforme ; complete/cancel/free protocolaires | callback data live ; producer progressant |
| runtime de binding | state publié avant poll ; repoll ; extraction/free exactly-once | runtime cible schedule le repoll |
| host fonctionnel | cohérence duplex entre les handles élémentaires | binding, demand et peer/timeout progressent |

Le terminal global reste un théorème de composition ; il ne devient pas une garantie propre de la
projection simplement parce qu’elle possède les handles.

## 7. Comparaison des choix

**Résumé.** Le callback direct vers un trampoline officiel est retenu parce qu’il minimise l’état
central et réutilise le flow control déjà armé par operation. Le waitable réduit les reverse calls
et uniformise davantage Java/Python, au prix d’une queue native, d’un dispatcher et de nouveaux
lifecycles ; il reste l’alternative documentée si un binding ne peut pas respecter le contrat V1.

**Intention.** Ce tableau explique le choix et les triggers qui imposeraient de le rouvrir. Il ne
demande pas une implémentation complète de chaque catégorie avant de livrer la V1.

| Critère | Callback C direct | Callback C via dispatcher | Waitable W | Future handles |
|---|---:|---:|---:|---:|
| Exemples représentatifs | MsQuic, libuv handles/requests | GIO `GTask`/`GMainContext`, Node-API thread-safe functions | gRPC Core, WASI waitable set | FoundationDB C API, UniFFI async |
| Reverse FFI | oui | oui, thread contrôlé | non sur le chemin normal | wake callback possible |
| Java 17 | complexe | maîtrisable | favorable | moyen |
| .NET Framework 4.8 | faisable | faisable | favorable | favorable |
| Python/subinterpreters | complexe | complexe au shutdown | favorable | moyen |
| C++ callback-native | favorable | favorable | correct | favorable |
| Batching | difficile | possible | naturel | faible |
| Reentrancy | interdite dans le trampoline, sauf fault latch | contrôlable | commands non reentrant | poll callback |
| Shutdown | callback barrier | drain du thread | drain de queue | drain des futures |
| Code central Rust | faible au départ | moyen | plus élevé | moyen |
| Code par binding | trampoline petit mais language-specific | moyen/élevé | dispatcher moyen | moyen |
| Garantie host décisive | trampoline fini, rooted, no-user-code | thread attaché, drainé et joint | routing/release/drain | repoll/extract/free |
| Assumption réellement résiduelle | runtime host callable, CPU | thread/runtime host obtient du CPU | dispatcher obtient du CPU | repoll obtient du CPU |
| Mesure critique | callback latency | queue + upcall | wait/batch latency | poll crossings |

Le callback C direct vers le trampoline officiel est le lowering V1. Les bindings .NET et Java
doivent démontrer la garantie de retour, le rooting et le shutdown sans user code sur le native
thread dans leur propre modèle de refinement et leurs tests. L’échec de cette démonstration, une
pause host incompatible avec la santé des workers Tokio ou un besoin mesuré de batching sont des
triggers de réouverture vers un dispatcher natif ou W. Le callback applicatif arbitraire et le
`spawn_blocking` par event restent exclus.

## 8. Projections par langage

**Résumé.** Le runtime d’isolation doit être invisible pour l’utilisateur applicatif. Chaque
projection garantit la traduction du même event protocol vers les primitives de son
langage, sans exposer les native threads ni les borrowed pointers.

**Intention.** L’utilisateur du package cible doit retrouver une API proche de
`armonik-transport`, pas apprendre l’ABI. Le runtime de binding absorbe le routing, les roots et les
releases ; la projection fonctionnelle absorbe la sémantique de stream et de cancellation.

### 8.1 .NET

Un delegate statique rooted entre dans un trampoline qui résout
`consumer_token -> OperationState`, valide l’event, copie ou retain son payload, publie un
`CompletionRecord` dans une `ConcurrentQueue`, signale le worker host puis retourne une disposition.
Le worker, et non l’upcall, complète la `TaskCompletionSource`. Toutes les TCS utilisent en plus
`RunContinuationsAsynchronously`. Toute exception est interceptée et convertie en
`OPERATION_FAULT` ou `RUNTIME_FAULT`; elle ne traverse jamais le frame P/Invoke.

Le cœur du worker est volontairement petit :

```csharp
while (await signal.WaitAsync())
{
    while (completions.TryDequeue(out var completion))
    {
        try { registry[completion.Token].Complete(completion); }
        catch (Exception error) { FailBinding(completion.Token, error); }
        finally { completion.ReleaseOwnedPayload(); }
    }
}
```

`SafeHandle` protège les handles synchrones. L’operation state reste enregistrée jusqu’à
`COMPLETED`, pas seulement jusqu’à `Dispose`. La projection peut fournir un `HttpMessageHandler`
optionnel pour `Grpc.Net.Client`, tout en conservant une API transport testable indépendamment.

### 8.2 Java 17

Le thread natif est attaché à la JVM et le state est conservé par global reference. Le
trampoline ne doit **pas** appeler directement `CompletableFuture.complete` : un dependent stage
non-async peut être exécuté par le thread qui complète la future. Il valide et copie/retain donc
l’event, le dépose dans une concurrent queue prépubliée, signale un dispatcher Java ou poste vers un executor
dont le contrat interdit l’exécution inline, puis retourne. Un `Executor` arbitraire ne suffit pas,
car son interface autorise une exécution sur le calling thread.

`Flow.Publisher` traduit `request(n)` en armements de read bornés. L’implémentation doit décider si
une read correspond à un item ou si elle agrège pour respecter les attentes de `ByteBuffer` ; ce
choix appartient à la projection, pas à l’ABI.

Le chemin FFM Java 22 pourra remplacer le lowering JNI sans changer le protocole.

### 8.3 Python

Un trampoline appelé sur un foreign thread ne lance jamais directement du Python. Une petite couche
native copie/retain l’event, le publie dans une queue associée à l’interpreter, effectue un signal
thread-safe borné vers la bonne loop et retourne sans attendre le GIL. La projection acquiert le GIL
plus tard, utilise `loop.call_soon_threadsafe` et release le payload après consommation. Si une
version de CPython ne permet pas cette garantie, ce binding doit proposer un lowering distinct ; il
ne ralentit pas implicitement les autres cibles.

Le shutdown doit terminer avant la finalization de l’interpreter. Le binding doit associer un
runtime à l’interpreter concerné et ne pas supposer un unique interpreter global.

### 8.4 C++11

Le wrapper fournit RAII, une MPSC queue et l’intégration à un executor injecté. Le trampoline C++
capture toute exception, publie un `CompletionRecord` owned, signale ou appelle seulement un `post`
documenté non-inline, puis retourne. Le worker résout la `std::promise`; aucun callback applicatif
n’est invoqué inline.

### 8.5 C++20

Un awaiter enregistre le `coroutine_handle` dans l’operation state. Le worker du binding le poste
vers l’executor et ne reprend pas la coroutine inline. La lifetime du
coroutine frame et la race completion/cancellation restent une garantie locale du type `task` de
la projection, sous l’assumption que l’executor accepte puis exécute le post.

### 8.6 Contrats des projections

| Couche observée | Garanties propres | Assumptions reçues |
|---|---|---|
| runtime de binding | layout/capabilities validés ; registration/routing/release ; roots et finalization emboîtées ; trampoline C retourne après publication locale bornée | ABI conforme ; contexte host utilisable ; queue/executor host progressent ; host déclenche shutdown |
| projection idiomatique | outcome → future/task ; scheduler explicite ; demand, request trailers et cancellation projetés | runtime de binding fidèle ; executor accepte le post |
| runtime du langage | roots déclarées respectées ; travail posté rendu éligible selon son contrat | OS schedule le runtime |
| application | respecte l’API safe et ne peut pas rompre la safety interne par un handler ordinaire | user code retourne seulement pour les propriétés applicatives qui l’attendent |

## 9. Modèle TLA+, points de vue et proof obligations

**Résumé.** La formalisation doit précéder le gel de l’ABI et modéliser séparément Rust fonctionnel,
Rust FFI, runtime de binding et host fonctionnel. Chaque couche fournit des garanties de safety et
de liveness sous des assumptions explicites. La preuve finale doit décharger les assumptions de
chaque couche par les garanties des autres ; seules des assumptions générales peuvent rester.

**Intention.** Le résultat recherché n’est pas seulement « TLC ne trouve rien ». Il faut pouvoir
expliquer, pour chaque point de vue, ce qui est garanti, quelle fairness est supposée, quelle couche
fournit cette fairness et pourquoi la composition ne repose pas sur un cycle de promesses vide.

### 9.1 Découpage des modules et relation de refinement

Le modèle est organisé par point de vue et par protocole transverse :

```text
Transport.tla                 machine fonctionnelle abstraite
CallLifecycle.tla             parenté fonctionnelle et conditions de destruction
FlowControl.tla               demand read/write, trailers et bounds

RustFunctional.tla            futures/streams, connection et call côté Rust
RustFfi.tla                   boundary abstraite commands/notifications/owners
FfiRuntime.tla                gates, children, tasks, in-flight callback et join
CallbackProtocol.tla          upcall, résultat, payload et callback barrier
HostQueue.tla                 publication, signal et consommation host
HostFfi.tla                   roots et projection du lowering choisi
HostFunctional.tla            futures/streams idiomatiques visibles par l’application
DotNetBinding.tla             refinement .NET du Host FFI
JavaBinding.tla               refinement JNI/Java du Host FFI
Cpp11Binding.tla              refinement C++11 du Host FFI
Cpp20Binding.tla              refinement coroutine du binding C++11
PythonBinding.tla             refinement CPython du Host FFI

HostCalls.tla                 protocole transverse RustFunctional <-> RustFfi <-> HostFfi <-> HostFunctional
Shutdown.tla                  fermeture transverse de tous les objets et threads
FailureContainment.tla        gates, quarantaine, join et recovery boundary
WaitableCounterModel.tla      alternative informative, hors chemin de conformance V1

Composition.tla               assume/guarantee des quatre couches
Refinement.tla                HostFunctional + couches cachées => Transport
MC*.tla                       configurations TLC finies et traces litmus
```

`HostCalls.tla` et `Shutdown.tla` ne sont pas des annexes indépendantes. Ils étendent les quatre
machines de couche, importent `CallLifecycle.tla` et participent au refinement : une host-call reply
peut rendre une call fonctionnelle progressable ; un shutdown abstrait n’est atteint que lorsque
runtime, connections, calls, events, invocations, queue, dispatcher et roots satisfont leur
condition de quiescence. `FailureContainment.tla` raffine une rupture interne en
`TransportError(ProtocolViolation)` puis interdit toute nouvelle observation fonctionnelle de
l’instance fautive.

`CallbackProtocol.tla` raffine l’action abstraite `DeliverNotification` de la V1 et doit prouver
`TrampolineReturns` à partir des actions du Host FFI, sans fairness du user code. Chaque module de
binding raffine ensuite les actions `Publish`, `Signal`, `Consume` et `Shutdown` avec les primitives
réelles de son runtime. `WaitableCounterModel.tla` raffine la même notification uniquement pour
expliquer l’alternative et identifier les triggers de réouverture ; il n’est pas une proof
obligation d’une implémentation V1.

### 9.2 État et cycles de vie modélisés

L’état fonctionnel abstrait contient :

```text
transportState    New | Open | Closing | Closed
connectionState   New | Connecting | Open | Draining | Closed
callState         New | Open | Cancelling | Completed
sendState         Open | WritePending | Closed
recvState         BeforeHeaders | Open | ReadPending | Ended
headers           None | Value | Error
requestTrailers   None | Pending | Sent | Error
trailers          None | Value
outcome           None | Ok | Error
consumerDemand    Nat
```

L’état concret ajoute :

```text
runtimeState      Running | Stopping | Draining | Stopped |
                  Quarantining | FailedDraining | FailedQuiescent | FailedUnquiesced
bindingGate       Open | Closed
nativeGate        Open | Closed
operations        token -> concrete state
commands          sequence
callbackRecords   operation -> Empty | Published | Consumed
ownedPayloads     owner -> RustOwned | HostLease | Released
borrows           object -> set of borrowers
callbackInFlight  set of operation IDs
trampolineState   operation -> Idle | Entered | Published | Returned
hostCalls         invocation -> Pending | Replied | Failed | Cancelled
roots             set of host roots
producerRefs      Nat
hostQueueState    Open | Closing | Drained | Freed
hostWorkerState   New | Running | Joining | Joined
nativeTasks       set of task IDs
firstFault        None | fault record
```

Chaque objet a une action d’allocation, des borrows ou transferts nommés, une demande de close et
une action de destruction. Par exemple, `FreeHostQueue` exige `hostQueueState = Drained`,
`hostWorkerState = Joined` et `producerRefs = 0`; `FreeRuntime` exige en plus l’absence de root,
de callback record non consommé, de host call et de native task. La connection logique reste live tant qu’une
call la référence ; le transport reste live tant qu’une connection ou une call existe. Ces
prédicats rendent testable la table de lifecycle de la section 2.4.

Les variables `callbackInFlight`, `trampolineState` et `hostQueueState` sont actives dans la V1. La
host queue est une primitive du binding qui reçoit des `CompletionRecord` déjà copiés ou retained ;
ce n’est pas une completion queue native exposée par l’ABI. Les variables `nativeEvents`,
`waitableState` et `dispatcherState` n’existent que dans `WaitableCounterModel.tla`. Le modèle commun
ne doit pas transférer leurs fairness assumptions au chemin retenu.

### 9.3 Contrat de chaque point de vue

Pour chaque couche `C`, on distingue quatre ensembles : safety preconditions reçues,
`Fairness_C`, `SafetyGuarantees_C` et `LivenessGuarantees_C`. Le contrat a la forme :

```text
Spec_C /\ SafetyPreconditions_C
    => SafetyGuarantees_C

Spec_C /\ SafetyPreconditions_C /\ Fairness_C
    => LivenessGuarantees_C
```

La fairness ne doit pas servir de prémisse à la safety : un scheduler injuste peut empêcher le
progress, mais il ne doit pas autoriser un double release ou un mauvais routing.

#### 9.3.1 Rust fonctionnel

- Safety preconditions : les buffers transmis par Rust FFI respectent leur borrow ; les commands
  correspondent à une call/connection live.
- Fairness assumptions : Tokio/OS repoll une task réveillée ; le peer produit un résultat ou un
  timer local expire ; un host call requis reçoit reply/fail/cancel.
- Safety guarantees : state machine HTTP/2 valide, au plus une pending read/write selon le contrat,
  errors réseau confinées à la call/connection, aucun isolation ID dérivé du peer.
- Liveness guarantees : sous ces assumptions, une operation obtient un outcome ; cancellation et
  deadline deviennent finalement observables sans promettre de rollback distant.

#### 9.3.2 Rust FFI

- Safety preconditions : les plages de raw memory annoncées par un downcall sont réellement
  readable pendant l’appel et l’address space n’est pas déjà corrompu. Les valeurs, handles,
  tailles et flags restent non fiables et sont validés par Rust FFI ; un stale handle ordinaire
  doit produire une erreur, non un undefined behavior.
- Fairness assumptions : Rust fonctionnel termine ou réagit à cancel ; le trampoline officiel
  retourne. Cette dernière assumption doit être déchargée par la garantie locale du Host FFI.
- Safety guarantees : validation avant déréférencement, transfert d’owner exactement une fois,
  gate natif fermé sur quarantaine, aucune destruction avant quiescence et aucun callback après la
  callback barrier.
- Liveness guarantees : une command acceptée produit un event ou un terminal ; shutdown/quarantaine
  réveille les waiters et atteint une barrière ou un état explicite `FailedUnquiesced`; les
  downcalls classés synchrones terminent sans dépendre du dispatcher ou du peer.

#### 9.3.3 Runtime de binding / Host FFI

- Safety preconditions : les events produits par Rust FFI satisfont layout, generation et owner
  contractuels ; le runtime du langage conserve les roots demandées.
- Fairness assumptions : communes, l’OS laisse progresser le thread entré dans le binding et une
  allocation bornée retourne succès ou échec ; le runtime accepte le signal/post qui suit la
  publication. Aucune de ces assumptions ne porte sur le retour d’un handler utilisateur.
- Safety guarantees communes : registration avant start, input pointers live pendant chaque
  downcall, roots conservées jusqu’à la barrière et aucune continuation utilisateur inline sur le
  chemin de notification. Le trampoline ajoute validation/copie/retain, publication dans la host
  queue et retour sans user code.
- Liveness guarantees : un callback accepté retourne après la transition locale ; toute operation
  connue reçoit une completion terminale lors du shutdown ou d’une faute ; le worker/dispatcher
  host éventuel est joint avant libération de sa queue.

#### 9.3.4 Host fonctionnel

- Safety preconditions : l’application utilise l’API safe, ne conserve pas un lease après dispose
  et respecte la concurrence documentée.
- Fairness assumptions : le consumer demande, annule ou ferme finalement ; ses handlers et host
  services reviennent ; son executor/event loop continue à tourner.
- Safety guarantees : projection exacte vers `Task`/`CompletableFuture`/async stream/coroutine,
  ordering fonctionnel, cancellation non assimilée à rollback, aucun raw handle ou borrowed pointer
  exposé.
- Liveness guarantees : sous ces assumptions et les garanties de Host FFI, toute observation
  fonctionnelle pending reçoit valeur, erreur, cancellation ou terminal stream ; close attend la
  barrière requise.

La cible d’architecture est de déplacer les preconditions spécifiques dans les wrappers safe et de
transformer les silences externes en outcomes locaux par timeout/cancellation. Les assumptions
résiduelles souhaitées sont : CPU finalement accordé au process, ressources finies disponibles ou
échec explicite, timer local progressant, code utilisateur commencé qui revient, et absence de
corruption mémoire préalable.

### 9.4 Invariants

Les invariants principaux sont :

```text
NoUnknownOwner
NoDoubleRelease
NoEventWithoutOperation
AtMostOnePendingRead
AtMostOnePendingWrite
TerminalIsLast
NoHostReplyAfterResolution
RequestTrailersSentAtMostOnce
NoWriteAfterRequestComplete
StoppedImpliesNoInFlightDelivery
UnknownOperationImpliesBindingGateClosed
QuarantineImpliesNativeGateClosed
FirstFaultIsImmutable
QuarantineNeverReturnsToRunning
DestroyImpliesQuiescent
KnownOperationsFailAtMostOnce
QueueFreedImpliesDispatcherJoined
ParentOutlivesChildren
BorrowImpliesOwnerLive
TombstonesAreBounded
PeerCannotChooseIsolationIds
```

Le modèle callback V1 ajoute `NoRecursiveCallbackUnlessEnabled`,
`NoDestroyWhileCallbackInFlight`, `AllowedReentrantCommandsOnly`,
`CallbackStatePreRegistered`, `PublishedBeforeTrampolineReturn`,
`NoUserActionInsideTrampoline` et `BorrowValidUntilTrampolineReturn`.

### 9.5 Fairness profiles et liveness

Les fairness assumptions sont attachées aux actions qui les nécessitent : weak fairness de
`RustStep`, `ConsumeHostRecord`, `PostCompletion`, `HostReply`, `ConsumerDemand`, `TimerFire` et
`JoinHostWorker` seulement dans les profiles où l’environnement correspondant coopère. Aucune
fairness du peer n’est ajoutée dans le profile avec deadline ; `CallbackReturn` reste séparé dans
le modèle callback applicatif arbitraire utilisé comme contre-exemple. Dans la V1, la transition
abstraite du Host FFI est :

```text
EnterTrampoline
  -> ValidateEnvelope
  -> CopyOrRetainPayload | LatchFault
  -> PublishOperationState
  -> SignalHostExecutor
  -> Return(ACCEPTED | OPERATION_FAULT | RUNTIME_FAULT)
```

Le modèle de refinement peut détailler ces états pour les races et failure paths, mais le lowering
abstrait les regroupe en une action finie `DeliverViaTrampoline`. Il ne contient ni
`RunUserHandler` ni attente d’une action Rust future. `TrampolineReturns` est donc une garantie du
Host FFI sous la fairness résiduelle générale accordant du CPU au thread déjà entré ; ce n’est pas
une fairness assumption sur le code applicatif.

```text
SafetyOnly
CooperativeConsumer
SilentPeerWithTimeout
ResponsiveHostService
ShutdownWithDrain
DirectTrampoline              \* Return appartient à la transition Host FFI
ArbitraryCallbackMayBlock       \* contre-modèle, non candidat
UnknownOperationQuarantines
QuarantineJoinSucceeds
QuarantineJoinTimesOut
MaliciousPeerIsOperationLocal
```

Une timeout ne prouve pas que le peer ou le callback termine. Elle garantit que le composant local
cesse d’attendre, à condition que `TimerFire` et le code qui traite la deadline soient fair.

TLA+ ne prouve pas qu’une fonction compilée est réellement finie, qu’un allocator ou le GC respecte
une borne temporelle, ni qu’un `Executor` concret n’exécute pas inline. La preuve du modèle fixe le
contrat et exclut les cycles protocolaires ; la conformance de `DeliverViaTrampoline` exige en plus
revue du trusted code, tests d’injection d’exception et de saturation, et vérification propre à
chaque runtime (`RunContinuationsAsynchronously` en .NET, post garanti non-inline en Java).

### 9.6 Composition des assumptions et garanties

Pour les couches `C_i`, la proof obligation de composition est :

```text
pour chaque i : ResidualSafety /\ SafetyGuarantees_des_autres => SafetyPreconditions_i
pour chaque i : ResidualFairness /\ LivenessGuarantees_des_autres => Fairness_i

alors : ResidualSafety /\ ResidualFairness /\ Composition(Spec_i)
        => (∧_i SafetyGuarantees_i) /\ (∧_i LivenessGuarantees_i)
```

Cette règle n’autorise pas une circularité gratuite. Si `A` garantit son progress en supposant le
progress de `B`, et réciproquement, la preuve doit exhiber une action initialement enabled, un
variant décroissant ou une fairness externe qui casse le cycle. TLC explore les cycles finis ; les
lemmes de composition non finies pourront être prouvés avec TLAPS ou argumentés dans le document
normatif.

Exemples d’obligations à fermer :

| Assumption reçue | Garantie qui doit la décharger |
|---|---|
| Rust fonctionnel suppose qu’un host call est résolu | Host fonctionnel garantit reply/fail/cancel, acheminé par Host FFI et Rust FFI |
| Rust FFI callback suppose que le trampoline retourne | Host FFI garantit une transition bornée, sans user code ni continuation inline |
| Rust FFI suppose que la queue retrouve de la capacité | Host FFI garantit drain tant que le runtime est actif, ou déclenche shutdown/quarantaine |
| Host FFI suppose qu’un event transféré a un owner valide | Rust FFI garantit validation et transfert atomique |
| Host fonctionnel suppose qu’une call native atteint un terminal | Rust fonctionnel + Rust FFI + Host FFI garantissent outcome, transport et routing sous fairness résiduelle |
| Rust FFI suppose qu’un dispatcher cesse avant `FreeQueue` | Host FFI garantit `Joined` avant le dernier release de runtime |

La livraison du modèle inclura une table exhaustive `assumption -> guarantee -> module -> test ou
preuve`. Toute ligne sans garantie devient une assumption résiduelle publiée, pas une phrase vague
de liveness.

### 9.7 Refinement fonctionnel, host calls et shutdown

Le refinement masque queue, handles, owners et dispatcher :

```text
concrete RESPONSE_HEADERS delivered -> abstract headers becomes Value
concrete READ_DONE delivered         -> abstract response stream yields bytes
concrete COMPLETED delivered         -> abstract outcome becomes terminal
concrete callback Entered/Returned    -> stuttering autour de Published
concrete queued but undelivered      -> abstract stuttering
concrete event_release               -> abstract stuttering
concrete host-call request/reply     -> stuttering, puis effet fonctionnel autorisé
concrete shutdown quiescent          -> abstract Transport/Connection becomes Closed
concrete protocol quarantine         -> abstract ProtocolViolation terminal
```

`Shutdown.tla` prouve à la fois une propriété fonctionnelle — les closes obtiennent un terminal —
et une propriété de lifecycle — plus aucun child ou borrow avant destroy. `HostCalls.tla` prouve
exactly-once et la résolution par shutdown/cancel, puis le refinement cache l’invocation si elle
n’a pas d’observation fonctionnelle propre. `FailureContainment.tla` interdit toute livraison après
le fault latch, mais autorise les releases et joins nécessaires à la quiescence.

Le modèle ne prouve pas qu’un pointer C réel est valide, qu’un module natif n’écrit pas arbitrairement
dans le heap, que les diagnostics n’exfiltrent rien, ni qu’un compiler préserve le modèle en
présence d’undefined behavior. Ces obligations restent aux runtime checks, à la revue `unsafe`, aux
sanitizers/fuzzers et, pour du code non fiable, à l’isolation de process ou sandbox.

### 9.8 Traces et tests de conformance

Chaque counterexample pertinent devient une trace exécutable : completion avant le retour de
start ; cancel contre completion ; close de connection avec plusieurs calls ; release pendant une
command ; queue pleine pendant shutdown ; host reply contre cancellation ; callback reentrant ;
callback bloqué ; unknown/stale token ; wrong-runtime/double release ; fault au milieu d’un batch ;
join juste avant/après deadline ; `FreeQueue` tenté avant `JoinDispatcher`; input borrow conservé
après downcall ; peer malveillant incapable de choisir un isolation ID ; recovery limitée par
quota. Les tests contrôlent les barrières pour imposer le même partial order sans prétendre
reproduire le scheduler exact.

### 9.9 Matrice unique des fournisseurs de garanties

Cette matrice contient uniquement des garanties **locales et observables à la sortie d’une
couche**. Une croix ne marque ni l’auteur du code ni le scheduler qui participe au mécanisme. Les
propriétés end-to-end n’y figurent pas : elles sont prouvées par composition juste après.

| Garantie locale observable | Nature | Rust fonctionnel | Rust FFI | Host FFI | Host fonctionnel | Assumptions reçues | Assumption déchargée chez le consumer |
|---|---|---:|---:|---:|---:|---|---|
| Une transition ready réveille le waker enregistré | safety fonctionnelle | × |  |  |  | state et waker live | le driver Rust peut attendre un wake fidèle |
| Un input peer, cancel ou timer **observé** devient un outcome interne ; un input invalide reste operation/connection-local | safety fonctionnelle | × |  |  |  | parser/state intègres | Rust FFI peut attendre un outcome cohérent |
| Un event publié est conforme et son owner natif est transféré au plus une fois | safety |  | × |  |  | command et state valides | Host FFI peut consommer l’event |
| Une command acceptée devient finalement notification ou terminal | liveness |  | × |  |  | outcome Rust, CPU ; trampoline retourne | Host FFI peut attendre une résolution native |
| La barrière native implique zéro task et zéro upcall natif futur | safety |  | × |  |  | producers natifs suivent leur lifecycle | Host FFI peut libérer ensuite ses roots |
| State/root sont publiés avant start et restent live jusqu’à la barrière | safety |  |  | × |  | runtime host respecte les roots déclarées | Rust FFI peut utiliser le token/root annoncé |
| Pour C, le trampoline ne lance aucun user code, contient l’exception et retourne après une transition locale | safety + liveness |  |  | × |  | event conforme ; thread obtient du CPU ; allocation retourne succès/échec | Rust FFI reçoit `TrampolineReturns` |
| Pour W, un event retiré est routé ou quarantined puis released exactement une fois | safety + liveness |  |  | × |  | event conforme ; dispatcher obtient du CPU | Host fonctionnel reçoit une observation fidèle |
| Demand, cancellation et `complete(RequestTrailers)` deviennent des commands valides | safety fonctionnelle |  |  |  | × | API safe respectée ; binding accepte ou refuse explicitement | Rust FFI reçoit le flow control |
| Toute observation native terminale met les objets fonctionnels dans un état terminal exactement une fois | safety |  |  |  | × | event conforme ; state host live | l’application reçoit futures/streams cohérents |

La ligne waitable décrit uniquement `WaitableCounterModel.tla`. Elle n’est pas une obligation du
chemin V1 et n’entre pas dans sa composition.

Les propriétés externes ne reçoivent pas de croix dans cette matrice. Du point de vue du système
ArmoniK, elles restent des assumptions résiduelles tant qu’un contrat externe ne les garantit pas :

| Assumption résiduelle | Propriétés qui en dépendent | Réduction possible |
|---|---|---|
| une task ou continuation éligible obtient finalement du CPU | progress de toutes les couches concernées | health checks, process supervision ; aucune garantie in-process absolue |
| un timer armé produit finalement une observation locale | timeout et cancellation bornée | timer monotone local et tests de suspension/reprise |
| le peer répond ou ferme | outcome sans deadline locale | deadline transforme le silence en timeout local |
| un handler utilisateur commencé retourne | liveness qui attend explicitement ce handler | ne jamais l’inclure dans le reactor ou la barrière native ; deadline/isolation si nécessaire |
| une allocation finie retourne succès ou échec | trampoline, copie et publication | quotas, préallocation des slots, failure result explicite |

| Propriété composée | Garanties et fairness nécessaires |
|---|---|
| une call commencée obtient un outcome host | outcome Rust + notification Rust FFI + trampoline + host queue consumption + CPU + peer-or-timeout |
| aucune donnée n’est livrée après unknown operation | validation/gate Host FFI + quarantine/gate Rust FFI + absence d’upcall après barrière |
| destruction totale sûre | quiescence native + trampoline/dispatcher quiescent + roots/events released + parent outlives children |
| shutdown termine | close host + commands Rust FFI + terminaison Rust + drain du lowering + CPU ; user handler seulement si shutdown l’attend explicitement |

Cette séparation corrige notamment le cas Tokio : ArmoniK garantit le wake correct ; « Tokio/OS
repoll finalement » reste une assumption externe dans le contrat ArmoniK, même si l’implémentation
du runtime fournit le mécanisme de scheduling. La propriété end-to-end utilise les deux sans
attribuer artificiellement la state machine ou sa liveness globale à Tokio.

## 10. Performance : hypothèses et protocole de mesure

**Résumé.** Le benchmark doit comparer la baseline callback au design callback complet, puis aux
bindings officiels qui ajoutent leur host queue et leur scheduler. Il mesure le système entier et
isole les coûts sans utiliser un microbenchmark de function call comme substitut du streaming
concurrent, du GC, du shutdown et des fault paths.

**Intention.** Le message principal est que la performance est une condition d’acceptation mesurée,
pas une propriété déduite du nombre de crossings. Les variantes doivent inclure les mécanismes de
safety, de lifecycle et de shutdown nécessaires en production.

### 10.1 Hypothèses à tester

H1. Le callback direct réduit la latency d’un event isolé.

H2. Un seul signal host peut permettre au worker du binding de drainer plusieurs
`CompletionRecord`, même si chaque observation native provoque encore un callback.

H3. La copie vers un managed array domine les différences de notification pour les petits/moyens
body chunks ; un owned native payload est plus intéressant pour les grands chunks.

H4. Un `spawn_blocking` par event augmente la tail latency et la consommation de threads sous
callbacks lents.

H5. Un dispatcher dédié offre une variance plus prévisible qu’un pool partagé.

H6. Java 17 paie davantage que C++ le coût et le lifecycle des upcalls JNI ; ce coût peut déclencher
un dispatcher natif ou, en dernier recours, la réouverture de l’alternative waitable.

H7. La validation `event_id + generation`, les tombstones bornés et le fault latch ont un coût
mesurable mais faible devant le crossing FFI ; batch release doit éviter que le durcissement double
le nombre de downcalls.

H8. Le trampoline direct a une latency plus faible en régime nominal, mais une pause GC/JVM peut
allonger directement le stall d’un worker Tokio.

H9. L’armement unitaire borne naturellement les callbacks fonctionnels en vol et évite qu’une queue
générique apporte un bénéfice avant une forte concurrence ou un besoin de batching.

Ces hypothèses peuvent être invalidées ; elles ne doivent pas être formulées comme des conclusions
du design.

### 10.2 Variantes comparées

1. ABI callback actuelle comme baseline ;
2. callback V1 vers trampoline minimal avec payload copié ;
3. chemin complet .NET : trampoline, `ConcurrentQueue`, signal et completion sur executor ;
4. chemin complet Java : JNI, global refs, concurrent queue, signal et `CompletableFuture` ;
5. C++11/C++20 et Python lorsque leurs bindings sont disponibles ;
6. callback push via dispatcher natif dédié seulement si une cible le requiert ;
7. owned payload optionnel face à la copie.

### 10.3 Workloads

- unary sans body significatif ;
- server streaming petits messages ;
- client streaming ;
- bidirectional streaming ;
- body chunks petits, moyens et grands ;
- 1, 16, 256 et plusieurs milliers de calls concurrents selon la machine ;
- cancellation storms ;
- slow consumer et queue proche de sa capacité ;
- logger saturé ;
- shutdown avec operations et host calls en vol ;
- injection de duplicate/stale IDs et fault storms, uniquement via hooks de test ;
- recovery répétée jusqu’au quota ou circuit breaker.

### 10.4 Mesures

- throughput utile ;
- p50, p95, p99 et max latency par operation et par event ;
- allocations Rust et managed ;
- bytes copiés ;
- native/managed crossings ;
- wakeups, context switches et threads ;
- queue depth et batch size ;
- CPU par cœur, cache misses si l’outillage le permet ;
- temps de cancellation et de shutdown ;
- temps `fault detected -> start gate closed -> FailedQuiescent/FailedUnquiesced` ;
- mémoire retenue par tombstones et runtimes quarantined ;
- GC pauses et pinned/native memory high-water mark.

### 10.5 Critère de décision

Le seuil n’est pas fixé dans ce document. L’équipe doit le définir avant les résultats pour éviter
un choix opportuniste. La V1 est acceptée seulement si les proofs et tests ferment
`TrampolineReturns`, rooting, exception containment et shutdown sur .NET et Java, et si le chemin
complet satisfait les SLOs. Un échec ne choisit pas automatiquement W : il ouvre un ADR ciblé entre
dispatcher natif, mécanisme propre à la cible et waitable. Les seuils X %, Y µs et les pauses
GC/JVM admises doivent être fixés à partir des SLOs ArmoniK.

### 10.6 Artefacts nécessaires à la décision

La décision exige une instrumentation native et host corrélable, un harness reproductible, les
workloads de la section 10.3, les résultats bruts et les seuils d’acceptation fixés avant mesure.
Cette liste n’est pas une attribution de fairness : l’organisation de l’équipe et la validation des
SLOs sont des décisions de gouvernance distinctes du contrat temporel.

## 11. Versioning et capability negotiation

**Résumé.** Une ABI additive n’est sûre que si un ancien consumer peut libérer et ignorer les
nouveaux éléments sans manquer une action obligatoire. Les capacités doivent être négociées dans
les deux sens dès que le comportement du consumer est concerné.

**Intention.** Le lecteur doit retenir qu’un changement est additif seulement si un ancien binding
peut conserver safety et liveness. La possibilité de décoder une nouvelle valeur ne suffit pas si
elle introduit un payload, un reply ou un rearm obligatoire.

`ak_abi_version()` informe le binding sur la bibliothèque. Le runtime doit aussi connaître les
capabilities supportées par le binding : event kinds compris, host services fournis, payload modes,
batching, wake handle et diagnostics.

```c
ak_status ak_runtime_create(const ak_runtime_options *options,
                            const ak_capability_set *consumer_capabilities,
                            ak_capability_set *negotiated_capabilities,
                            ...);
```

Règles :

- une capability obligatoire non commune fait échouer create ou start synchroniquement ;
- un event inconnu reste releasable génériquement ;
- aucun event inconnu ne requiert un reply, un rearm ou un changement d’ownership ;
- les structs extensibles portent `size` et éventuellement `version` ;
- les constants existantes ne changent jamais de valeur ou de sens ;
- les messages d’erreur restent diagnostics, les codes portent le contrôle ;
- les reason codes de quarantaine et la sémantique de `join` sont versionnés comme partie du
  protocole, pas comme simples diagnostics ;
- un ancien binding sans failure containment négocié ne peut pas ouvrir un runtime qui produirait
  des owned events ou host calls dont il ne sait pas assurer la sortie.

## 12. Plan de remédiation et de décision

**Résumé.** La stack est corrigée de bas en haut. Le runtime fonctionnel est inséré sous la FFI ;
les fondations d’identity et de lifecycle sont corrigées avant le client ; le callback et chaque
binding arrivent ensuite avec leur modèle de refinement. Le détail PR par PR est normé dans le
[plan de remédiation](PR_REMEDIATION.md).

**Intention.** La remédiation doit réutiliser la state machine existante sans préserver ses mauvais
couplages. Aucun compatibility shim ou second protocole waitable n’est ajouté au-dessus de la
branche actuelle.

### Phase 0 — contrat sémantique et actions TLA+

Écrire les observations fonctionnelles et les actions communes : configuration, connection
logique, plusieurs calls, request/response headers, demand, body chunks, request/response trailers,
cancel, timeout et terminal. Un event avant le retour de `start` est permis parce que le consumer
token est enregistré avant l’appel. La liste V1 des downcalls reentrant ordinaires est vide.

Définir en même temps threat actors, scopes de faute, unknown/stale/wrong-owner token,
`FailedQuiescent`, `FailedUnquiesced`, shutdown et conditions de destruction. Les sorties sont
`Transport.tla`, `CallLifecycle.tla`, `FlowControl.tla` et les premières traces litmus.

### Phase 1 — runtime fonctionnel Rust sous #744

Après #743, remplacer la finalité de #746 par une PR `armonik-transport::runtime`. Y déplacer la
machine fonctionnelle récupérée de #747 et de la branche reactor. Le runtime expose des Rust types
et streams, pas des C callbacks. Il implémente `complete(RequestTrailers)` dès l’introduction du
request body et documente connection logique versus sockets physiques lazy.

### Phase 2 — fondations FFI dans #744

Amender #744 avec un `ak_runtime` explicite, des registries scopées et generation-tagged, les
ownership classes, le start gate, la callback barrier, la quarantaine et le join. Distinguer le
runtime d’isolation du moteur Tokio éventuellement process-wide. Écrire `FfiRuntime.tla`,
`GenerationalIds.tla`, `MemoryOwnership.tla` et `FailureContainment.tla` dans cette phase.

### Phase 3 — client et lowering callback

Restructurer #747 comme projection du client/connection fonctionnel et rattacher chaque child à son
runtime. Ouvrir ensuite une PR issue des commits reactor utiles pour le lowering callback : token
prépublié, payload borrowed puis copié/retained, callback result, host queue, absence de user code,
fault path et callback barrier. `CallbackProtocol.tla`, `HostQueue.tla` et les tests de conformance
font partie du même diff.

### Phase 4 — bindings .NET et Java

Implémenter d’abord le binding .NET Framework 4.8 dans le package fonctionnel : delegate rooted,
`ConcurrentQueue`, signal, `TaskCompletionSource` sans continuation inline et shutdown joint. Faire
de même en Java 17 : global refs JNI, queue, attach/detach, `CompletableFuture`, exception JNI
containment et JVM shutdown. Chaque PR inclut son module de refinement et les fault-injection tests
correspondants.

Il n’y a pas de spike waitable symétrique obligatoire. Un dispatcher natif ou W n’est prototypé que
si un binding échoue à garantir `TrampolineReturns`, si une pause host compromet les workers Tokio
ou si un SLO mesuré le justifie.

### Phase 5 — documentation normative et stabilisation

Stabiliser `TRANSPORT.md + PROTOCOL.md + DESIGN.md` lorsque signatures, actions TLA+ et tests se
correspondent. Exécuter conformance tests, fuzzing, fault injection, revue `unsafe`,
sanitizers/Miri/Loom lorsque pertinents, `-Xcheck:jni`, GC stress et benchmarks multi-plateformes.
Relier chaque invariant à un runtime check, un test ou une assumption explicitement non vérifiable.
Ce travail suit les pratiques du [NIST SSDF](https://doi.org/10.6028/NIST.SP.800-218), sans présenter
le framework comme une preuve d’absence de vulnérabilité.

### Phase 6 — C++11, C++20, Python et seconde FFI

C++11 valide queue, RAII, promise/future et join ; C++20 projette le même binding en coroutines sans
changer l’ABI. Python valide interpreter identity, GIL scheduling et finalization. Une seconde FFI
ArmoniK mesure ensuite ce qui est réellement factorisable ; c’est seulement alors que le noyau
d’isolation peut devenir un package générique.

## 13. Questions ouvertes soumises à l’équipe

**Résumé.** Les points suivants changent réellement le contrat ou le coût ; ils ne doivent pas être
résolus implicitement par la première implémentation.

**Intention.** Ces questions sont les décisions qui empêchent aujourd’hui une preuve complète ou
un benchmark interprétable. Les laisser ouvertes dans le code reviendrait à les confier aux races
du premier prototype.

1. Quels p99, throughput et budget CPU/allocation doit satisfaire le chemin complet de chaque
   binding ?
2. Quel quota global de calls actives borne les callback records et la host queue ?
3. À saturation de la host queue, le trampoline utilise-t-il un slot préalloué, retourne-t-il
   `RUNTIME_FAULT`, ou quarantine-t-il directement le runtime ?
4. Les body payloads managés sont-ils copiés en V1, avec owned lease ajouté ensuite par capability ?
5. Les diagnostics utilisent-ils un ring buffer lossy séparé de toute notification fonctionnelle ?
6. Quelles host services bidirectionnelles sont réellement nécessaires en V1 ?
7. Le shutdown a-t-il une timeout contractuelle, et quelle policy d’unload suit
   `FailedUnquiesced` ?
8. Les operation tokens utilisent-ils 128 bits, ou un index et une generation 64 bits avec quelle
   preuve de non-wrap/reuse ?
9. Quel scope de quarantaine minimise le blast radius : runtime, client, connection ou tenant ?
10. Après `FailedUnquiesced`, le host doit-il retain jusqu’à process exit, fail-fast, ou isoler le
    runtime dans un worker process killable ?
11. Quel quota de runtimes quarantined et quelle circuit-breaker policy évitent un restart storm ?
12. Le threat model inclut-il une bibliothèque native malveillante ? Si oui, l’isolation in-process
    est exclue quel que soit le lowering.
13. Le callback retourne-t-il `ACCEPTED/OPERATION_FAULT/RUNTIME_FAULT`, ou une disposition plus
    détaillée sans rendre le callback reentrant ?
14. Les pauses GC/JVM observées sur le trampoline sont-elles compatibles avec la santé des workers
    Tokio, ou certaines cibles exigent-elles un callback dispatcher dédié ?
15. Le moteur Tokio partagé interdit-il l’unload dynamique de la DLL, ou faut-il un lifecycle
    process-wide explicitement joinable ?
16. Le type public doit-il s’appeler `Connection` lorsqu’il représente un pool lazy de connexions
    physiques, ou faut-il exposer séparément `Transport`, `Client` et `Connection` ?

## 14. Conclusion retenue pour la V1

**Résumé.** Le problème n’est pas de choisir entre « callback » et « polling » en abstraction. Il
est de placer les obligations de scheduling, ownership et liveness là où elles peuvent être
implémentées une fois, prouvées au niveau du design et mesurées sur les runtimes réellement
supportés.

**Intention.** La cible est une API fonctionnelle simple reposant sur une composition presque
fermée de garanties entre Rust et le host. Le mécanisme V1 est fixé afin d’avancer ; ses triggers de
réouverture restent explicites et mesurables.

Le callback C direct vers un trampoline officiel est le design V1. Le host binding garantit
`TrampolineReturns`. Il est plus simple parce qu’il :

- réutilise l’armement read/write comme bound naturel des observations en vol ;
- évite une completion queue, un polling thread et leurs lifecycles pour le chemin nominal ;
- ne fait jamais entrer une continuation ou un handler utilisateur dans le scheduler Rust ;
- ferme la fast-completion race par publication du state avant start ;
- peut signaler `ACCEPTED`, `OPERATION_FAULT` ou `RUNTIME_FAULT` au point exact de l’upcall ;
- reste une petite machine formalisable à la manière des contrats de callbacks MsQuic.

Ce résultat ne justifie ni un callback applicatif ni `spawn_blocking(callback)`. Le trampoline fait
partie du runtime de binding et de la trusted computing base. Il doit être fini, exception-safe,
rooted, sans lock cyclique et sans user code. Le modèle TLA+ doit prouver que sa garantie décharge
l’assumption correspondante de Rust FFI.

Le waitable W reste une alternative documentée, pas un second chemin livré. Il ne redevient une
option d’implémentation que si Java/Python ne peuvent pas fournir le trampoline à coût et lifecycle
acceptables, si les pauses du runtime host compromettent les workers Tokio, ou si batching et
centralisation du drain répondent à un besoin produit mesuré. WASI reste une source utile pour la
séparation interface/lowering et les waitable sets ; ce n’est pas une obligation d’architecture
pour une DLL native.

Le plan de PR en découle directement : conserver #743, amender #744, sortir #745 du chemin critique,
remplacer #746 par `armonik-transport::runtime`, restructurer #747 et scinder la branche reactor.
Les raisons et exit criteria sont détaillés dans [PR_REMEDIATION.md](PR_REMEDIATION.md).

Le failure containment réduit fortement le risque d’un mauvais routing, d’une lifetime ambiguë et
d’un cleanup dangereux. Il ne permet pas d’affirmer un « risque cyber nul » : contre du code natif
malveillant ou une corruption mémoire déjà active, la décision pertinente est l’isolation de
process/sandbox et sa mesure, non une variante plus complexe du même protocole in-process.
