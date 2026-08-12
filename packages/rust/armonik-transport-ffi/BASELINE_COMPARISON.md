# Comparaison de la FFI actuelle, du spike C# et du design cible

## Statut et méthode

**Résumé.** Ce document conserve le constat sur les prototypes afin que les documents de design ne
dépendent pas d’une branche appelée à disparaître. Il compare des révisions précises, distingue les
qualités déjà démontrées des propriétés encore absentes et sert de checklist de migration. Il n’est
pas un contrat normatif.

**Intention.** Le message principal est de préserver les résultats positifs des prototypes sans
confondre faisabilité et contrat de production. Chaque écart doit être rattaché à une garantie, un
lifecycle ou une proof obligation du design cible.

Baseline analysée :

- FFI Rust : commit `430566dcdc151ea77518353183c2db36486b72ea` sur
  `wk/feat/rust-ffi-reactor` ;
- spike C# : commit `49b388a2d71828a11f5b84146846849fbbfbcfcb` sur
  `wk/spike/ffi-http2-handler` ;
- design cible : décision V1 de [`DESIGN.md`](DESIGN.md), non implémentée ; sa transformation de la
  stack est détaillée dans [`PR_REMEDIATION.md`](PR_REMEDIATION.md).

Les conclusions doivent être réévaluées si ces révisions changent. Le spike C# est lu avec
`git show` ; il n’est pas nécessaire de conserver sa branche pour comprendre les constats ci-dessous.

## 1. Vue d’ensemble

**Résumé.** L’implémentation Rust actuelle démontre qu’un reactor par request peut préserver le
duplex et le flow control. Le spike démontre qu’on peut projeter cette ABI en `HttpMessageHandler`
pour `Grpc.Net.Client`, y compris sur .NET Framework. Ils ne constituent pas encore un contrat
multi-langage complet : publication du handle, reentrancy, liveness conditionnelle et shutdown
restent à résoudre.

**Intention.** Le lecteur doit retenir que la baseline valide le chemin fonctionnel, tandis que le
design cible ajoute la composition temporelle et les lifecycles nécessaires à plusieurs langages.

| Dimension | FFI Rust actuelle | Spike C# | Design cible V1 |
|---|---|---|---|
| Notification | callback direct depuis la request task | reverse P/Invoke, copie, TCS | trampoline officiel borné, host queue, signal puis retour |
| Unité de sérialisation | une task par request | état par `RustCall` | call fonctionnelle + operation registry scopée par runtime |
| Flow control | read/write armées | une TCS read/write | demand projeté, command/event explicites |
| Payload event | emprunté pendant callback | copié en `byte[]` | borrowed copié/retained par le trampoline avant retour |
| Fast completion | task spawn avant publication handle | `GCHandle` publié avant start | consumer token publié avant start |
| Reentrancy | code l’autorise, docs l’interdisent | rearm direct possible | aucun downcall ordinaire reentrant en V1 ; commands postées |
| Completion | terminal event par request | complète headers/read/write/tasks | refinement fonctionnel spécifié |
| Shutdown runtime | absent, runtime process-wide | absent | `Stopping/Draining/Stopped` + barrière |
| Rupture d’invariant | status/panic guard, pas de quarantaine | exception callback avalée | fault latch, quarantaine, join ou `FailedUnquiesced` |
| Java/Python | non démontré | non applicable | exigence de design |
| Formalisation | tests Rust, pas de modèle | checklist/tests spike | TLA+ + conformance traces |

## 2. FFI Rust actuelle

**Résumé.** Le cœur est compact et plusieurs décisions sont solides : ownership des handles, inputs
copiés lorsque nécessaire, emitter unique et terminal outcome convergent. Les problèmes principaux
sont à la frontière entre cette machine et ce que la documentation promet au consumer.

**Intention.** L’analyse sépare les propriétés réellement garanties par le code des assumptions que
le consumer doit aujourd’hui deviner. Cette séparation sert de point de départ au refinement.

### 2.1 Architecture observée

`ak_client_create` est synchrone : il parse la configuration, charge les certificats et construit
un `hyper_util::client::legacy::Client`, donc un connection pool. Il n’ouvre aucune socket. La
première request établit lazily une connection ; HTTP/2 peut multiplexer plusieurs requests sur la
même connection et le pool peut en ouvrir ou en remplacer d’autres. L’API actuelle choisit donc un
pool/client, pas une connection physique explicite.

Le handle client est alloué et enregistré par Rust. Une request en vol garde une counted `Arc` sur
le pool : `ak_client_release` retire la référence du caller, mais ne détruit ni le pool ni les
connections nécessaires aux requests existantes. La dernière référence Rust désalloue l’objet.
Cette propriété est solide, mais la baseline n’expose pas le lifecycle fonctionnel
`connect -> open call -> drain connection` que le design cible veut rendre visible.

`ak_request_start` :

1. valide le client et décode les headers ;
2. crée un body channel de capacité un ;
3. crée les flags d’armement et le command channel ;
4. spawn `Task::drive` sur le runtime Tokio global ;
5. insère ensuite le request handle dans la registry et le retourne.

Le driving loop effectue un `tokio::select!` biaisé entre commands, deadline, response headers,
write progress et response frames. `EventSink::emit` appelle directement la function pointer avec
un payload emprunté. Un seul driving loop émet les events d’une request, ce qui les sérialise pour
cette request ; plusieurs requests peuvent appeler simultanément le consumer.

Le runtime Tokio est un `OnceLock<Runtime>` process-wide, créé lazily et jamais arrêté. Cette
décision convient à un prototype chargé pour la vie du process, mais ne fournit pas de barrière
d’unload ou de remplacement de services host.

Les request inputs nécessaires après un downcall sont copiés dans des objets Rust. Les payloads de
callback sont borrowed uniquement pendant l’upcall et ne sont jamais libérés par C#. Les handles
client/request sont libérés par `ak_*_release`, c’est-à-dire par l’allocator Rust qui les a créés.

### 2.2 Ce qui est déjà convaincant

- Les entry points sont panic-guarded.
- Les handles sont reference-counted et les calls concurrents gardent une counted reference.
- Les request input buffers sont copiés avant le retour lorsqu’ils doivent survivre au downcall.
- Les callback payloads ont une lifetime courte clairement représentée dans le code.
- Un emitter unique rend l’ordre par request analysable.
- Les flags read/write sont relâchés avant émission, ce qui évite un deadlock lors d’un rearm.
- Le body channel de capacité un propage du backpressure.
- La plupart des chemins convergent vers un unique terminal outcome.
- Les tests forcent plusieurs races de release, cancel, timeout et peer reset.

Ces qualités doivent être préservées même si le lowering change.

### 2.3 Course entre start et premier event

La task est spawn avant l’insertion et l’écriture du native request handle. Rien dans le scheduler
ne garantit qu’elle ne produira pas un event avant le retour de `ak_request_start`.

```text
C#                       Rust                         Tokio
 | ak_request_start       |                            |
 |----------------------->| spawn Task::drive          |
 |                        |--------------------------->|
 |                        |                            | callback(ctx)
 |                        | insert handle              |
 |<-----------------------| return handle              |
```

Le spike enracine son `GCHandle` avant start, ce qui évite un dangling `ctx`, mais un binding qui
indexe son état par native handle seulement après le retour perdrait l’event. Le futur contrat doit
soit interdire tout event avant le retour par une barrière, soit faire choisir et publier un
consumer token avant start.

### 2.4 Contradiction sur reentrancy

Le contrat écrit de la baseline interdit de re-enter la request depuis son callback. Le code libère
pourtant explicitement les arm flags avant `WRITE_DONE` et `READ_DONE`, avec des commentaires disant
que le consumer est autorisé à armer l’opération suivante depuis le callback.

Ce n’est pas une nuance documentaire. Le binding C# naturel rearme une read en réponse à la read
précédente. La décision doit être prise au niveau du contrat :

- callbacks reentrant avec downcalls autorisés et non blocking ; ou
- callbacks non reentrant dont le binding doit sortir via sa propre queue ; ou
- protocole pull sans upcall.

### 2.5 Liveness conditionnelle de `COMPLETED`

Le response body n’est pollé que si `read_armed || drain_response`. `drain_response` devient vrai
notamment lorsque le command channel disparaît ou lorsque le send side échoue. Tant que le request
handle reste vivant, qu’aucune read n’est armée et qu’aucune timeout ne force la terminaison, un peer
peut avoir terminé le response stream sans que le driving loop observe sa fin.

La phrase « `COMPLETED` arrive sur tous les chemins » décrit la convergence des chemins exécutés par
le loop. Elle ne constitue pas la garantie inconditionnelle :

```text
Start => <>Completed
```

La propriété correcte dépend du consumer :

```text
Start /\ ConsumerEventuallyReadsOrReleases /\ PeerEventuallyEnds
      => <>Completed
```

Cette hypothèse est la clause de flow control absente du contrat actuel.

### 2.6 Ordering des races terminales

Le document dit correctement que des events sans relation causale n’ont pas d’ordre promis. Les cas
visibles doivent néanmoins être énumérés :

- cancel concurrent de timeout : `CANCELLED` ou `TIMEOUT` ;
- clean trailers concurrentes d’un reset : succès ou transport error selon l’observation gagnante ;
- cancellation concurrente d’une clean completion : résultat terminal déjà choisi ou cancellation.

Les tests doivent vérifier l’ensemble autorisé, et le binding ne doit pas convertir une issue
possible en « impossible state ».

### 2.7 Versioning additif incomplet

`ak_abi_version()` informe le consumer sur la bibliothèque. La bibliothèque ne connaît pas les
capabilities du consumer. Un nouvel event n’est safely ignorable que s’il est releasable
génériquement, sans payload possédé inconnu, action requise, armement ou terminal semantics.

Un changement qui demande une réaction du consumer requiert une négociation bidirectionnelle, pas
seulement un ABI version getter.

### 2.8 Contrats observés et lacunes

La baseline ne fournit pas de chaîne assume/guarantee pour ses propriétés temporelles. La lecture
utile part de chaque composant et sépare ses garanties effectives de ce qu’il suppose :

| Composant observé | Garanties effectives | Assumptions reçues | Lacunes |
|---|---|---|---|
| `armonik-transport` | configuration et connector cohérents ; aucun scheduling FFI promis | le caller poll les futures qu’il construit | ce composant n’own ni reactor FFI ni callback |
| reactor `armonik-transport-ffi` | emitter unique par request ; flags libérés avant event ; handle présent validé ; terminal unique sur les chemins exécutés | Tokio repoll ; callback retourne ; consumer maintient le flow control ; peer finit ou timeout | pas de shutdown ; ABA d’adresse ; pas de quarantine |
| binding | `ctx` et callback restent live ; payload borrowed consommé avant retour | reactor respecte ordering et terminal ; runtime host reste utilisable | aucun contrat commun ne force ces garanties pour un nouveau binding |

Le CPU finalement accordé, le retour d’un handler et la réponse du peer — ou l’expiration d’une
deadline locale — restent des assumptions résiduelles. Elles ne sont les garanties d’aucune des
trois couches observées ci-dessus.

Le dernier état est précisément un défaut de contrat : en l’absence de shutdown, la terminaison du
process est la seule barrière effective, sans garantie de quiescence exploitable par le host.

### 2.9 Error containment et risque de stale handle

Le code actuel a une défense utile : une raw pointer de handle sert uniquement de key dans une
registry d’`Arc`; elle n’est pas déréférencée directement. Une release concurrente ne détruit donc
pas l’objet qu’un autre downcall utilise, et un handle absent est rejeté. Les panic guards empêchent
également un unwind Rust de traverser la frontière C.

`src/handle.rs` documente toutefois une limite précise : si un handle est release puis si
l’allocator réutilise la même adresse pour un objet du même type, le stale pointer peut résoudre le
nouvel objet. C’est un problème d’ABA que seule une identité avec generation ferme. Il ne suffit pas
de conserver plus longtemps une adresse dans un set ; il faut empêcher sa réinterprétation comme
une nouvelle capability.

La baseline ne possède pas d’`operation_id` central : le routing des events utilise le `ctx` opaque
fourni au start. Elle n’a donc pas aujourd’hui le cas exact « queue event avec operation inconnue »
de l’alternative waitable ; le cas analogue pour C est un `ctx`/token non résolvable par le trampoline.
Elle n’a pas non plus de mécanisme équivalent pour mettre le runtime Tokio
global en quarantaine, fermer le start gate, échouer toutes les requests et obtenir une barrière de
quiescence. Un panic de request est converti en terminal `AK_INTERNAL_PANIC`; une contradiction dans
une registry partagée n’a pas de failure containment protocol documenté.

Le peer réseau ne contrôle pas directement les adresses de handle ou `ctx`. Cette propriété doit
être préservée : une frame distante malformée doit rester une erreur de request/connection, faute de
quoi un peer pourrait transformer le futur mécanisme de quarantaine en denial of service global.

## 3. Spike C# `RustHttpHandler`

**Résumé.** Le spike valide l’intégration fonctionnelle la plus importante : `Grpc.Net.Client` peut
rester propriétaire du framing gRPC au-dessus d’un `HttpMessageHandler` alimenté par la FFI Rust.
Il contient de bonnes protections de lifetime, mais son error containment et plusieurs sémantiques
de stream ne sont pas prêts pour une bibliothèque publique.

**Intention.** Le lecteur doit distinguer les patterns à conserver — rooting, publication avant
start, continuations asynchrones — des lacunes structurelles que des `catch` locaux ne réparent pas.

### 3.1 Architecture observée

`RustCall` est à la fois wrapper d’une request native et demultiplexer :

- un delegate static est rooted pour la vie du process ;
- un `GCHandle` par request est alloué avant `ak_request_start` ;
- headers et completion ont leur `TaskCompletionSource` ;
- au plus une TCS read et une TCS write sont stockées ;
- chaque TCS utilise `RunContinuationsAsynchronously` ;
- le callback copie immédiatement le borrowed payload ;
- le `GCHandle` n’est libéré que sur failed start ou `COMPLETED` ;
- `Dispose` release la native request mais ne libère pas prématurément le contexte.

Le payload natif borrowed est copié dans un `byte[]` alloué et désalloué par le managed heap avant
le retour du callback. Le spike n’appelle donc jamais `free`/`delete` sur de la mémoire Rust. Le
`GCHandle`, lui, appartient au binding et n’est libéré qu’après failed start ou `COMPLETED`. Cette
asymétrie correcte doit être généralisée au futur event owner et au shutdown, faute de quoi un
terminal logique pourrait encore précéder le dernier accès natif.

`RustHttpHandler` retourne la response dès les headers et pompe le request body en background. Ce
point est indispensable au client streaming et au bidirectional streaming : attendre la fin du
request body avant de rendre la response deadlockerait certains calls.

### 3.2 Résultats positifs du spike

- L’intégration `HttpMessageHandler` conserve `Grpc.Net.Client` pour framing, status, deadlines et
  retry.
- La projection est duplex.
- Le `GCHandle` ferme la fast-callback lifetime race sur `ctx`.
- La TCS est publiée avant l’armement read/write.
- Les continuations ne s’exécutent pas inline sur le callback thread.
- Le borrowed payload est copié avant le retour du callback.
- Le contexte survit à `Dispose` jusqu’au terminal event.
- Le single-threaded `SynchronizationContext` a été pris en compte dans le pump.

Ces points démontrent la faisabilité fonctionnelle et donnent des patterns réutilisables quel que
soit le lowering final.

### 3.3 Exception avalée dans le callback

Le callback top-level capture toute exception puis l’ignore. C’est nécessaire pour ne pas unwind
dans Rust, mais insuffisant pour la liveness : un `Blob.Decode`, une allocation ou une transition
inattendue peut échouer après consommation de l’event et laisser l’application attendre une TCS qui
ne sera jamais complétée.

Le binding de production doit disposer d’un failure channel local capable de :

1. enregistrer le binding failure ;
2. compléter exceptionnellement toutes les operations pending ;
3. demander cancellation native ;
4. conserver le contexte jusqu’au vrai terminal event ou shutdown barrier ;
5. signaler le défaut sans traverser l’ABI.

### 3.4 Concurrence des reads et writes

`read_` et `write_` sont des slots uniques remplacés avant chaque armement. L’ABI refuse un second
armement, mais deux callers managés peuvent courir avant que le premier downcall ait établi l’état
natif attendu. Le second peut écraser la TCS du premier ou produire un résultat difficile à
attribuer.

La projection publique doit soit sérialiser explicitement les calls concurrents, soit documenter et
faire échouer localement le second avant le downcall. Le message « delivery serialized par request »
ne signifie pas que les API calls du consumer le sont.

### 3.5 Sémantique de fin du stream

`ResponseStream` traite un `READ_DONE` de longueur zéro comme end-of-stream. Le contrat courant
porte pourtant l’end logique par `COMPLETED`, et un empty DATA chunk ne devrait pas être confondu
sans règle normative. La nouvelle projection doit représenter séparément :

- body chunk, éventuellement vide si autorisé ;
- end-of-stream ;
- terminal error ;
- trailers disponibles avant publication de la fin.

### 3.6 Cancellation et statuts synchrones

Les `CancellationToken` passés à `Stream.ReadAsync` et `Stream.WriteAsync` ne sont pas utilisés. Le
token du `HttpMessageHandler` annule la call entière, mais une cancellation locale de read/write
n’a pas de projection distincte.

`CloseSend` ignore également le status synchronement retourné par la FFI. Toute downcall pouvant
échouer doit soit propager immédiatement son erreur, soit expliquer pourquoi l’état terminal la
remplacera sans laisser de waiter bloqué.

La limitation est aujourd’hui dans la commande FFI, pas dans le body channel Rust :
`Command::CloseSend` ne porte aucune donnée et l’implémentation termine en droppant le sender, alors
que `http_body_util::channel::Sender` expose déjà `send_trailers(HeaderMap)`. La remédiation naturelle
est donc `complete(RequestTrailers)`, avec une collection vide pour conserver le comportement
actuel. La completion de cet envoi doit être distinguée de la terminal completion de la call.

### 3.7 Resolution d’une write pending par completion

Sur successful `COMPLETED`, le spike complète une write encore pending avec `true`. Cela peut
assimiler « request terminée proprement côté response » à « chunk accepté par la connection ». Le
contrat actuel dit que completion résout les operations encore armées, pas nécessairement qu’elle
leur attribue un succès opérationnel.

Le modèle fonctionnel doit distinguer :

- acknowledgement `WRITE_DONE` ;
- terminal call outcome ;
- write abandonnée parce que le peer a fermé ou que la call s’est terminée.

### 3.8 Absence de shutdown

Le `RustHttpHandler.Dispose` release le client. Le runtime Rust reste process-wide et aucune barrière
ne garantit que tous les callbacks et requests sont terminés avant unload ou fin de services host.
Cette absence est acceptable pour un go/no-go spike ; elle bloque une API bidirectionnelle et une
bibliothèque unloadable.

### 3.9 Contrats observés du spike

| Composant observé | Garanties effectives | Assumptions reçues | Lacunes |
|---|---|---|---|
| FFI Rust | events sérialisés par request ; terminal sur les chemins exécutés | trampoline retourne ; reads continuent ou request released | pas de barrière runtime ni de quarantaine |
| `RustCall` | state/root publiés avant start ; payload copié ; TCS connues complétées avec continuations async | event et `ctx` conformes ; allocations/TCS réussissent | catch vide ; concurrence read/write incomplètement fermée |
| `RustHttpHandler` | request/response duplex ; cancellation de call propagée | pumps obtiennent du CPU ; peer ou timeout progresse | pas de request trailers ; status de close parfois ignoré ; pas de shutdown global |
| BCL/`Grpc.Net.Client` | framing/status/retry gRPC restent au-dessus du transport | handler respecte `HttpMessageHandler` et stream semantics | ne peut réparer une TCS stranded dans le binding |
| runtime .NET | roots déclarées respectées ; continuations postées rendues éligibles selon le scheduler | OS lui accorde du CPU | aucune garantie de terminaison du user code |

Le retour du user code et la réponse du peer restent des assumptions externes du spike ; aucune
croix ou fusion avec le runtime .NET ne les transformerait en garanties de celui-ci.

### 3.10 Rupture d’invariant dans le callback

`OnEvent` reconstruit le `RustCall` par `GCHandle.FromIntPtr(ctx).Target`, copie le borrowed payload,
puis route selon `kind`. Tout le bloc est dans un `catch` vide. Cette forme protège la frontière
reverse P/Invoke d’une exception managée, mais elle ne contient pas fonctionnellement la faute : un
`ctx` stale, un payload impossible ou une exception de decode perd l’event et laisse potentiellement
une ou plusieurs `TaskCompletionSource` pending sans fin.

Un event kind inconnu est ignoré par le `switch`. C’est compatible avec le contrat courant parce
que le payload est borrowed et ne fuit pas au retour. Cela reste sûr seulement tant que le nouvel
event n’exige ni release, acknowledgement, rearm ni terminal transition. Le spike n’émet ni metric
ni diagnostic permettant de distinguer une extension additive d’une rupture de protocole.

Il n’existe pas de registry globale des `RustCall`, de start gate ou de runtime handle auquel
rattacher une quarantaine. Le spike ne peut donc pas appliquer « première faute latchée, aucun
nouvel appel, toutes les tasks connues échouent, join ». Ajouter seulement un `catch` qui appelle
`Environment.FailFast` serait excessif pour une erreur locale et insuffisamment fondé pour une
corruption mémoire. Le runtime de binding proposé doit porter cette décision et laisser au host la
politique retain, worker restart ou process fail-fast.

## 4. Écarts vers le design cible

**Résumé.** La remédiation ne consiste pas à réécrire le spike ligne par ligne. Il faut d’abord
stabiliser les sémantiques qui déterminent l’architecture, puis déplacer les obligations communes
dans un runtime de binding.

**Intention.** Chaque ligne doit disparaître par une décision normative, une garantie implémentée et
un test/refinement associé — pas seulement par un changement de signature.

| Écart | Risque actuel | Remédiation de design |
|---|---|---|
| start/event non ordonnés | event avant handle connu | consumer token enregistré avant start |
| reentrancy contradictoire | binding conforme aux docs mais non fonctionnel | règle normative et modèle séparé |
| liveness de completion implicite | waiter sans nouvelle read | fairness consumer explicite ou drain/cancel |
| borrowed payload callback | copie obligatoire immédiate | copie/retain garanti dans le trampoline avant publication host |
| no callback shutdown barrier | late upcall/use-after-free | runtime lifecycle et terminal barrier |
| exception callback avalée | task managée stranded | failure channel local + cancel + terminal cleanup |
| handle pointer sans generation | ABA : stale handle vise un nouvel objet | ID opaque indexé + generation/non-wrap |
| aucune quarantaine runtime | continuation après invariant partagé rompu | fault latch, start gate fermé, abort + join |
| pas d’état après join timeout | cleanup ou unload dangereux | `FailedUnquiesced`, retain/worker kill/fail-fast host |
| diagnostics non structurés sur faute | secret/log injection ou incident invisible | reason codes, record borné et sanitised |
| read/write concurrency | TCS écrasée | local operation state et rejection atomique |
| empty chunk = EOS | truncation possible | event/sémantique distincte |
| request close sans trailers | capacité HTTP/2 perdue | `complete(RequestTrailers)`, empty autorisé |
| status `CloseSend` ignoré | erreur perdue | propagation synchrone ou command acknowledgement |
| version unidirectionnelle | nouveau comportement incompatible | capability negotiation bilatérale |
| plomberie transport-specific | duplication future FFI | module `armonik_transport::runtime`, extraction après une seconde FFI |

## 5. Ce qu’il ne faut pas perdre

**Résumé.** Une nouvelle architecture plus générique ne doit pas effacer les résultats positifs des
prototypes. La correction n’est pas synonyme d’abstraction maximale.

**Intention.** La migration doit conserver le duplex, le backpressure et les patterns de lifetime
déjà validés ; une nouvelle abstraction qui les masque sans preuve serait une régression.

À conserver :

- un seul owner logique de l’émission par request ou une sérialisation équivalente ;
- backpressure read/write explicite ;
- input ownership simple au retour du downcall ;
- buffers Rust reference-counted lorsque le zero-copy natif est utile ;
- `RunContinuationsAsynchronously` côté .NET ;
- publication du state avant armement ;
- terminal event unique pour libérer les roots ;
- séparation HTTP/2 versus gRPC ;
- `HttpMessageHandler` comme façade d’intégration facultative ;
- tests de races qui acceptent un ensemble de résultats autorisés.

Le design V1 ne doit pas conserver le `catch` vide ni exposer le user code au callback. Il doit
aussi remplacer le `ctx` par request par un `runtime_ctx` rooted et une résolution du
`consumer_token` dans la registry : un token inconnu peut alors fermer le gate sans déréférencer un
objet d’operation stale. La host queue est bornée par les slots et le quota de calls actives ; elle
ne devient pas une completion queue native exposée par l’ABI.

## 6. Ordre recommandé de migration

**Résumé.** La décision de notification est prise ; la migration doit maintenant corriger la stack
au niveau où chaque abstraction apparaît, puis relier le code aux modèles commun et par binding.

**Intention.** L’ordre évite une PR de hardening au sommet d’une fondation incorrecte. Le détail et
les critères de fermeture sont dans [PR_REMEDIATION.md](PR_REMEDIATION.md).

1. Conserver #743 et insérer ensuite `armonik-transport::runtime` avec le contrat fonctionnel et les
   transitions récupérées du reactor.
2. Fermer/remplacer #746, dont les reexports plaçaient la machine au mauvais niveau.
3. Amender #744 avec runtime d’isolation explicite, generation-tagged IDs, ownership, shutdown et
   failure containment.
4. Restructurer #747 comme projection C du client/connection fonctionnel.
5. Scinder la branche reactor entre machine Rust et lowering callback ; ajouter les modèles TLA+ et
   traces de conformance dans les mêmes PR.
6. Implémenter la host queue et le scheduler .NET, avec un refinement TLA+ par binding.
7. Stabiliser l’ABI après fault injection, GC stress, revue security et benchmarks complets.
8. Valider Java, Python et C++11/C++20, Python et une seconde FFI avant toute extraction générique.

## 7. Verdict

**Résumé.** L’ABI et le spike ont rempli leur rôle : ils ont démontré la faisabilité du transport
duplex et révélé les vraies questions de contrat. Ils ne doivent pas être promus tels quels. La
V1 conserve le callback, mais remplace sa target applicative par un trampoline officiel et sépare
la machine fonctionnelle du lowering C.

**Intention.** Le message principal est « conserver la preuve de faisabilité, remplacer le contrat
implicite ». La prochaine valeur produite est une composition de garanties et de lifecycles, pas un
callback supplémentaire.

La FFI actuelle est proche de la famille MsQuic par son event callback direct, mais sans le même
shutdown protocol ni la même spécification de reentrancy. Le spike C# prouve que ce modèle est
utilisable. La couche supplémentaire n’a pas besoin d’être un runtime complexe : le trampoline
copie/retain, enfile un `CompletionRecord`, signale et retourne ; le worker host complète plus tard
les futures. La difficulté restante est concentrée dans identity, rooting, exception containment et
shutdown, qui justifient un modèle de refinement par binding. Le waitable n’est rouvert que si une
cible ne sait pas fournir ces garanties.
