# État de l’art des FFI asynchrones

## Statut et intention

**Résumé.** Ce document rassemble les modèles scientifiques et les retours d’expérience utiles pour
concevoir une FFI native asynchrone entre Rust et plusieurs langages. Il ne spécifie pas l’ABI
ArmoniK et ne prend pas seul la décision d’architecture. Il établit ce que les différentes familles
savent garantir, les hypothèses qu’elles reportent sur leur environnement et les coûts qu’elles
font porter aux bindings.

**Intention.** Le message principal est qu’une FFI asynchrone doit être choisie comme un protocole
composable entre plusieurs runtimes, et non comme une calling convention isolée. L’état de l’art
sert à rendre les choix et leurs limites discutables par l’équipe ; il ne désigne pas mécaniquement
un vainqueur.

Le lecteur visé connaît la programmation concurrente, les memory models et TLA+. Le vocabulaire
technique reste donc en anglais. Les termes *safety*, *liveness*, *fairness*, *callback*, *future*,
*completion queue*, *buffer*, *handle* et *runtime* ne sont pas traduits.

Les sources normatives ou primaires sont privilégiées. Les références à WASI décrivent le
Component Model et son Canonical ABI ; elles ne prétendent pas que ce dernier soit directement une
ABI C native.

## 1. Résumé exécutif

**Résumé.** Il n’existe pas de primitive universelle qui transforme une opération Rust asynchrone
en opération idiomatique et sûre dans tous les langages cibles. Les solutions éprouvées se rangent
principalement dans trois familles : callback direct, future handle, completion queue ou waitable
set. Elles peuvent exposer la même API fonctionnelle mais répartissent différemment le scheduling,
l’ownership, le backpressure et le shutdown.

**Intention.** Le lecteur doit retenir qu’il faut d’abord décider quelles garanties fonctionnelles
et temporelles ArmoniK veut fournir, puis choisir le lowering FFI qui permet de les implémenter dans
chaque langage cible avec le moins d’assumptions résiduelles possible.

Les résultats principaux de cette étude sont les suivants.

1. Une API fonctionnellement asynchrone n’impose pas une ABI à callbacks. Une completion queue peut
   être projetée en `Task`, `CompletableFuture`, coroutine Python ou coroutine C++20 sans exposer la
   file à l’utilisateur final.
2. Un callback direct minimise l’état intermédiaire dans la bibliothèque native et peut offrir une
   excellente latency. En contrepartie, chaque binding doit maîtriser reverse FFI, rooting,
   foreign threads, exception containment, reentrancy et shutdown. `spawn_blocking` protège les
   workers Tokio d’un callback lent ; il ne résout aucune de ces obligations.
3. Une completion queue déplace davantage de mécanique dans le runtime d’isolation Rust, mais rend
   le binding majoritairement downcall-only. Elle mutualise mieux le dispatcher et convient
   particulièrement à Java 17 et Python.
4. Un future handle est élégant pour une valeur terminale. Le transport étudié combine deux
   streams, des headers, des trailers, cancellation et backpressure ; il exige donc une machine
   d’état explicite même si chaque opération élémentaire est représentée par un future.
5. La bidirectionnalité fonctionnelle n’oblige pas davantage à utiliser des upcalls directs. Un log
   est une notification native vers le host ; un token provider est un host call corrélé avec une
   réponse. Les deux peuvent être encodés comme événements et commandes.
6. Toute propriété de liveness est conditionnelle. Ni Rust ni le binding ne peuvent garantir que le
   peer répond, que l’application continue à lire, qu’un callback retourne, ou qu’un event loop que
   l’application a arrêté continue à progresser. Ces conditions doivent être nommées, attribuées et
   modélisées.
7. Les théories de protocoles, typestates et session types donnent un vocabulaire et des résultats
   de composition puissants, mais leurs propriétés de progress supposent des endpoints conformes,
   un transport et un scheduling définis. Elles ne prennent pas en charge automatiquement le GC,
   l’unload d’une DLL, les exceptions étrangères ou les pointeurs empruntés.
8. TLA+ est adapté à la machine d’état et à ses hypothèses de fairness. TLC peut explorer les races
   du modèle fini. Cela ne prouve ni la conformité du Rust/C#/Java au modèle, ni les bornes de
   performance, ni la terminaison d’un callback arbitraire.
9. Une rupture d’invariant de routing n’est pas une erreur fonctionnelle ordinaire. La tolérer peut
   livrer un payload au mauvais consumer ou masquer un use-after-free ; arrêter tout le process à
   chaque input distant invalide crée en revanche un denial of service. Une architecture sûre doit
   donc classifier la faute, définir une unité de confinement et rendre sa sortie observable.
10. Si le callback cible exclusivement un trampoline officiel, sans user code, son retour peut être
    une garantie du runtime de binding plutôt qu’une assumption applicative. Cette composition rend
    une architecture MsQuic-style nettement plus simple que le waitable, sous réserve de valider
    reverse FFI, pauses du runtime host et shutdown dans chaque langage.

L’état de l’art suggère donc de séparer deux choix : l’API fonctionnelle, stable et idiomatique, et
le lowering d’isolation, remplaçable. Le [document de design](DESIGN.md) applique cette séparation à
ArmoniK et retient pour la V1 le callback MsQuic-style vers un trampoline officiel. Cette décision
n’est pas une conclusion universelle de l’état de l’art : elle dépend du bound par call, de la host
queue minimale et de la capacité des bindings officiels à garantir le retour sans user code.

## 2. Cadre scientifique : traces, points de vue et propriétés temporelles

**Résumé.** Une FFI asynchrone est un protocole entre machines d’état et schedulers, pas une simple
liste de signatures C. Une signature décrit les valeurs d’une transition ; elle ne décrit ni quand
la transition est permise, ni quelles transitions devront finir par se produire. Les contrats
doivent donc distinguer traces, safety, liveness, assumptions et garanties, et nommer la fairness
nécessaire à chaque propriété de progress.

**Intention.** Ce chapitre fixe le vocabulaire utilisé dans tout le document. Une garantie est ce
qu’une couche fournit à l’extérieur ; une assumption est ce qu’elle demande à son environnement.
Le même fait doit idéalement être une garantie de la couche voisine, afin que la composition ne
laisse que des assumptions générales et explicitement acceptées.

### 2.1 Le comportement observable est une trace

Un appel asynchrone peut être vu comme une trace d’actions :

```text
Start(token)
WriteArm(token, bytes)
WriteDone(token)
ReadArm(token)
ReadDone(token, bytes)
Cancel(token)
Completed(token, outcome)
```

Cette trace ne suffit pas sans relation causale. Par exemple, `ReadDone` doit correspondre à un
`ReadArm`, mais un `Cancel` concurrent d’un `Completed(OK)` peut perdre la race. Deux événements
simultanément disponibles peuvent ne pas avoir d’ordre contractuel. Le protocole doit donc définir
un ensemble de traces autorisées plutôt qu’un scénario nominal unique.

Les *communicating finite-state machines* de Brand et Zafiropulo ont établi très tôt l’intérêt de
modéliser les protocoles asynchrones par automates communicants, notamment pour détecter deadlocks,
réceptions non spécifiées et problèmes de boundedness [Brand et Zafiropulo, 1983](https://doi.org/10.1145/322374.322380).
Le résultat important pour notre problème est aussi une limite : une queue bornée, une queue non
bornée et un rendez-vous synchrone n’ont pas en général les mêmes traces ni les mêmes propriétés.

### 2.2 Point de vue, assumption et garantie

Un contrat n’a de sens qu’en nommant le composant observé. Pour un composant `C`, on écrit
schématiquement :

```text
Assumptions_C  =>  Guarantees_C
```

Une **garantie** porte sur les sorties et les actions que `C` contrôle et rend observables à son
environnement. Une **assumption** porte sur une action externe que `C` ne contrôle pas, mais dont il
a besoin pour établir sa garantie. Une assumption n’est donc ni une incertitude informelle ni une
garantie dégradée. C’est une dépendance contractuelle qui doit être satisfaite par la garantie d’un
autre composant ou rester explicitement résiduelle.

Par exemple, du point de vue du runtime de binding, « tout event accepté est finalement routé »
peut être une garantie sous l’assumption « le host scheduler exécute finalement le dispatcher ».
Du point de vue du host scheduler, cette dernière propriété est sa garantie. Inversement,
« l’application retourne de son handler » reste une assumption du host tant qu’aucune isolation ou
deadline ne la transforme en résultat observable.

Il faut éviter deux glissements fréquents :

- « le peer répond » n’est pas une garantie du transport ; c’est une assumption externe, ou une
  propriété que le transport remplace par la garantie locale « une deadline produit un timeout » ;
- « le consumer libère les events » n’est pas une garantie du runtime Rust si cette action est
  exécutée côté host ; le runtime peut en revanche garantir un quota, du backpressure et une
  fermeture déterministe lorsque cette assumption n’est pas satisfaite.

La cible d’architecture est plus exigeante qu’un partage symétrique des responsabilités : la
composition `Rust fonctionnel + Rust FFI + runtime de binding` doit faire le moins possible
d’assumptions sur le code applicatif. Elle doit fournir des garanties de bornage, de cancellation,
de terminalisation et de shutdown. Les assumptions qui subsistent devraient être générales : le
process reçoit du CPU, les allocations finies réussissent ou échouent explicitement, les handlers
utilisateur reviennent, et le peer répond ou une deadline expire.

### 2.3 Promises, futures et streams

Les *futures* ne sont pas d’abord une invention de Rust ou de .NET. Elles apparaissent comme objets
de calcul concurrent dans Multilisp [Halstead, 1985](https://doi.org/10.1145/4472.4478) ; les
*promises* séparent ensuite explicitement le producteur qui résout une valeur du consumer qui
l’observe [Liskov et Shrira, 1988](https://doi.org/10.1145/53990.54016). Une `Future<T>` représente
une occurrence terminale : elle passe de `Pending` à une valeur `T`, une erreur ou une cancellation,
et cette transition est observable au plus une fois.

```text
Promise endpoint : Resolve(T) | Reject(Error) | Cancel
Future endpoint  : Poll | RegisterWake | Await | Observe

Pending -> Ready(T) | Failed(Error) | Cancelled
Ready/Failed/Cancelled -> terminal
```

L’at-most-once est une safety guarantee du producer de la promise. `Pending => <>Ready` n’est pas
une propriété du type : elle exige une liveness guarantee du calcul producteur et la fairness du
scheduler qui l’exécute. Détruire ou annuler le future consumer n’implique pas non plus le rollback
du producer, sauf protocole supplémentaire.

Un stream asynchrone décrit autre chose : une séquence potentiellement non bornée d’éléments,
terminée par `Complete` ou `Error`, avec une relation de demand/backpressure. Les notations proches
ne sont donc pas équivalentes :

| Type conceptuel | Ce qui est séquencé | Scheduling et ordering | Backpressure et terminaison |
|---|---|---|---|
| `Future<T>` | une valeur terminale | un producer, une completion | pas de demand élémentaire |
| `Stream<Future<T>>` | des handles vers des calculs | les futures peuvent terminer hors ordre ; deux niveaux de scheduling | demander un handle ne borne pas forcément le travail déjà lancé |
| `Stream<Future<T[]>>` | des batches futurs | ordering entre batches et atomicité interne à définir | demand par batch ; taille, erreur partielle et mémoire restent à spécifier |
| `AsyncStream<T>` | des éléments produits au fil de la demand | ordering du stream défini par un seul protocole | demand, cancellation et terminal event appartiennent à la même state machine |

`Stream<Future<T>>` peut donc accumuler des opérations en vol même si le consumer ne consomme plus
leurs résultats. `Stream<Future<T[]>>` ajoute une unité d’atomicité et d’ownership : une erreur
porte-t-elle sur tout le batch, et qui libère les autres éléments ? Un `AsyncStream<T>` couple
normalement production et demand ; cela simplifie l’expression du backpressure, sans garantir à
lui seul que le producer ou le consumer sera schedulé. Reactive Streams formalise précisément ce
protocole de demand asynchrone et de queues bornées, mais laisse la fairness du scheduler à
l’environnement [Reactive Streams](https://www.reactive-streams.org/).

### 2.4 Safety

Une propriété de safety affirme que « quelque chose de mauvais n’arrive jamais ». Pour cette FFI,
les invariants candidats sont notamment :

- aucun event n’utilise un `operation_id` inconnu ou déjà libéré ;
- chaque buffer possédé est libéré exactement une fois ;
- un buffer emprunté n’est jamais lu après la fin de sa validité ;
- au plus une read et une write sont pending par request si le contrat le stipule ;
- `Completed` est terminal et émis au plus une fois ;
- aucune exception ou unwind ne traverse le frame C ;
- aucune callback ne commence après la barrière de shutdown ;
- une host-call reply correspond à un host call encore pending.

Une violation de safety possède un préfixe fini qui suffit à la démontrer. TLC est particulièrement
utile pour les interleavings qui conduisent à ce préfixe.

La safety ne doit pas être prouvée sous une fairness assumption : même si un scheduler cesse pour
toujours de choisir une action enabled, le système ne doit ni double-release, ni router au mauvais
consumer. La fairness n’intervient que pour établir le progress.

### 2.5 Liveness

Une propriété de liveness affirme que « quelque chose de bon finit par arriver ». Des formulations
utiles, mais nécessairement conditionnelles, sont :

```text
Started(op) /\ EnvironmentProgress(op) => <>Completed(op)
ReadArmed(op) /\ PeerEventuallyProduces(op) => <>ReadResolved(op)
ShutdownRequested /\ HostDrains => <>ShutdownComplete
HostCallIssued(id) /\ HostAnswers(id) => <>HostCallResolved(id)
```

La formule inconditionnelle `Started => <>Completed` est fausse dès que le consumer contrôle le
flow control, que le peer peut rester silencieux ou que l’opération appelle du code host arbitraire.
Une timeout interne peut transformer certaines assumptions en mécanisme, mais elle ne peut pas
interrompre proprement n’importe quel code étranger déjà en exécution.

### 2.6 Fairness

La fairness relie « action possible » et « action finalement choisie ». Dans la terminologie TLA,
la weak fairness exige qu’une action continûment enabled finisse par être prise ; la strong fairness
traite une action enabled infiniment souvent. Lamport souligne qu’ajouter une propriété de liveness
arbitraire peut introduire accidentellement de la safety ; les opérateurs de fairness rendent
l’hypothèse explicite [Lamport, *The Temporal Logic of Actions*](https://doi.org/10.1145/177492.177726).

Pour une FFI, il faut distinguer au minimum :

- **scheduler fairness** : une task réveillée est repollée ;
- **queue service fairness** : un event en file est finalement retiré ;
- **consumer fairness** : le consumer arme finalement la prochaine read ou write nécessaire ;
- **peer fairness** : le peer répond ou ferme finalement le stream ;
- **callback fairness** : un callback commencé retourne finalement ;
- **shutdown fairness** : le host continue à drainer jusqu’à la barrière terminale.

Ces assumptions ne sont pas interchangeables. Faire exécuter un callback dans `spawn_blocking` ne
transforme pas `CallbackReturns` en invariant du runtime ; cela change seulement le thread qui
attend.

### 2.7 Lecture assume/guarantee d’une chaîne complète

Une matrice de rôles masque le point de vue et confond facilement mécanisme, garantie et fairness.
La forme utile est un contrat par composant : ce qu’il contrôle vers l’extérieur, ce qu’il reçoit
de son environnement, puis la garantie voisine censée décharger cette assumption.

| Composant observé | Garantie fournie à l’extérieur | Assumption reçue | Décharge attendue |
|---|---|---|---|
| Rust fonctionnel | réveille le bon waker ; transforme peer/timeout en outcome interne | une task réveillée est repollée ; timer ou peer progresse | Tokio/OS pour le repoll ; timer local ou peer pour la terminaison |
| Rust FFI | transforme une command acceptée en event/terminal ; borne ownership et shutdown natifs | Rust fonctionnel progresse ; C : trampoline retourne ; W : capacité restituée | garanties du Rust fonctionnel et du runtime de binding choisi |
| Runtime de binding | enregistre avant start ; C : transition bornée et retour ; W : route/release ; conserve roots ; ferme son gate sur faute | events conformes ; C : contexte host utilisable ; W : dispatcher schedulé ; barrière native fidèle | Rust FFI pour les events/barrière ; runtime host/OS pour le progress nécessaire |
| Host fonctionnel | transforme futures/streams/cancellation en commands et observations idiomatiques | binding fidèle ; consumer demande, annule ou ferme | runtime de binding ; comportement applicatif résiduel |

Restent hors de cette chaîne les assumptions résiduelles `CPUEventuallyRuns`,
`UserCodeEventuallyReturns` et `PeerRespondsOrLocalDeadlineFires`. Elles ne deviennent des garanties
que si un composant externe explicite les fournit ; les placer dans une colonne « responsable » ne
créerait aucun contrat.

Tokio n’est donc pas « owner » de la state machine ArmoniK, et ArmoniK ne garantit pas le repoll par
Tokio. Le code Rust garantit le wake correct ; le runtime/OS fournit, ou non, la fairness qui rend
ce wake utile.

## 3. Pourquoi les théories connues ne donnent pas une solution prête à compiler

**Résumé.** Les automates, session types, promises, typestates et logiques temporelles couvrent des
facettes différentes. Leur combinaison guide une conception solide ; aucune ne résout seule la
traduction entre memory managers, schedulers, calling conventions et modèles d’annulation.

**Intention.** Le lecteur doit retenir que les résultats académiques sont des outils de
décomposition et de preuve sous assumptions explicites. Ils ne dispensent ni de modéliser les
lifetimes concrets de l’ABI, ni de prouver que les garanties d’une couche satisfont les assumptions
de la suivante.

### 3.1 Interface automata et contrats d’environnement

Les *interface automata* séparent les input assumptions des output guarantees et permettent de
raisonner sur la compatibilité temporelle de composants [de Alfaro et Henzinger,
2001](https://doi.org/10.1145/503209.503226). Cette distinction s’applique directement : « le binding
ne rappelle pas après `ShutdownComplete` » est une garantie ; « le consumer continue à drainer
jusqu’à `ShutdownComplete` » est une hypothèse.

La limite est que la compatibilité du modèle ne prouve pas que les implémentations suivent leurs
automates. Une raw pointer C, une exception JNI pending ou un `GCHandle` libéré trop tôt échappent au
vocabulaire si on ne les modélise pas explicitement.

### 3.2 Session types et projection

Les multiparty asynchronous session types expriment un protocole global, sa projection sur chaque
participant et des résultats de communication safety, progress et session fidelity [Honda,
Yoshida et Carbone, 2016](https://doi.org/10.1145/2827695). Ils justifient l’idée d’une spécification
fonctionnelle globale projetée en machines Rust et binding.

Leurs théorèmes de progress sont établis sous les hypothèses du calcul : endpoints bien typés,
sémantique des channels, règles de réduction et conditions de cohérence. Le host réel peut arrêter
son event loop, conserver un buffer emprunté, bloquer dans un logger ou unload la DLL. Ces actions
doivent être intégrées au modèle ou rester des obligations de l’environnement.

Le contre-exemple historique de Singularity est instructif. Les channel contracts et l’ownership
linéaire permettent une communication message-based efficace et sûre [Fähndrich et al.,
2006](https://www.microsoft.com/en-us/research/publication/language-support-for-fast-and-reliable-message-based-communication-in-singularity-os/).
Une analyse ultérieure montre toutefois que des processus fidèles à leur contrat local peuvent
encore deadlocker si le contrat global n’est pas réalisable [Stengel et Bultan,
2009](https://doi.org/10.1145/1572272.1572275). Une projection fidèle ne remplace donc pas une preuve
de liveness globale.

### 3.3 Limites de composition des promises et futures

La section 2.3 a défini promises, futures et streams. Leur intérêt théorique est de séparer la
production d’une occurrence terminale de son observation. Cette séparation facilite les règles
exactly-once et la propagation d’une valeur ou d’une erreur.

Elle ne spécifie pas qui poll, qui owns le payload, comment un stream exerce son backpressure, ni
ce que cancellation signifie après que l’effet distant s’est produit. Une future peut représenter
le résultat d’une state machine ; elle ne remplace pas cette machine. En particulier, composer des
futures élémentaires ne prouve pas le progress du protocole duplex qui les crée.

### 3.4 Callback typestates et lifestate

Les callback typestates étendent le typestate aux outputs asynchrones. DroidStar a montré qu’ils
peuvent être appris, et que cet apprentissage révèle des comportements surprenants ou non documentés
[Radhakrishna et al., 2018](https://www.microsoft.com/en-us/research/publication/droidstar-callback-typestates-for-android-classes/).
Lifestate unifie les API calls qui modifient l’état du framework et les callbacks que le framework
peut ensuite produire [Meier, Mover et Chang,
2019](https://doi.org/10.4230/LIPIcs.ECOOP.2019.1).

Cette famille formalise exactement la difficulté d’une API reentrant : un callback modifie par
downcall l’ensemble des callbacks futurs possibles. Elle n’impose toutefois ni calling convention,
ni stratégie de scheduling, ni memory ownership à travers une FFI.

### 3.5 Modèles de FFI et vérification dynamique

JNI Light formalise shared heap, cross-language calls, exceptions et GC pour un noyau JNI [Tan,
*JNI Light*](https://www.cse.psu.edu/~gxt29/papers/jnimodel.pdf). Jinn encode des milliers de règles
JNI et Python/C dans un petit ensemble de state machines et en synthétise des checkers dynamiques
[Lee et al., 2010](https://doi.org/10.1145/1806596.1806601).

Ces travaux rappellent deux limites : de nombreuses règles FFI sont contextuelles, et certaines ne
peuvent pas être entièrement vérifiées statiquement. Un protocole généré doit donc être complété par
des runtime checks, des tests de conformité et des diagnostics, sans présenter ces mécanismes comme
une preuve totale.

### 3.6 TLA+ et refinement

TLA permet d’exprimer dans le même formalisme une machine abstraite, une machine concrète et leur
relation de refinement. Son usage industriel montre sa valeur pour découvrir des bugs subtils au
stade du design [Newcombe et al., 2015](https://doi.org/10.1145/2699417).

Pour notre problème, TLA+ peut vérifier un modèle fini de :

- registration avant completion rapide ;
- read/write arming ;
- cancellation concurrente d’une completion ;
- callbacks ou dispatcher en vol pendant shutdown ;
- host calls bidirectionnels ;
- queue capacity et backpressure ;
- refinement entre transport fonctionnel et protocole d’isolation.

Ses limites doivent être écrites avec la même visibilité :

- TLC vérifie les instances et bornes explorées, pas toutes les tailles de buffer ou tous les
  nombres de requests ;
- la fairness est une hypothèse du modèle, pas une propriété mesurée du scheduler réel ;
- le modèle ne prouve pas que le code Rust, C# ou Java est une implémentation fidèle ;
- ni TLC ni TLAPS ne prouvent les latency percentiles, l’absence de cache miss ou le coût du GC ;
- un peer ou un callback arbitraire peut ne jamais terminer.

### 3.7 Fail-stop, fault containment et recovery

Les modèles *fail-stop* rendent une défaillance détectable et supposent qu’un composant fautif
cesse ensuite de produire des sorties [Schlichting et Schneider,
1983](https://doi.org/10.1145/357369.357371). C’est une abstraction utile pour un runtime FFI : une
fois un invariant de routing rompu, aucune nouvelle operation ne devrait être admise et aucun event
fonctionnel ne devrait atteindre l’application. Mais une DLL in-process ne devient pas fail-stop
par décret. Un thread corrompu peut continuer, une callback peut rester en vol et une raw pointer
invalide peut rendre le cleanup lui-même dangereux.

La *recovery-oriented computing* propose de redémarrer une unité plus petite que le process. Les
microreboots sont efficaces lorsque l’unité est restartable, que son état durable est externalisé et
que ses dépendances connaissent sa disparition [Candea et al.,
2004](https://www.usenix.org/conference/osdi-04/microreboot%E2%80%94-technique-cheap-recovery).
Ces hypothèses ne sont pas automatiquement vraies pour une FFI : le runtime natif, les roots GC, les
connections et les callbacks peuvent former une seule recovery unit. Une « recréation du handle »
sans barrière de quiescence n’est pas un microreboot ; c’est potentiellement un use-after-free.

Le résultat transférable est un protocole en trois temps : détecter et latch la première faute,
mettre l’unité en quarantaine, puis obtenir une barrière prouvant qu’elle est quiescent avant de la
détruire ou de la remplacer. Si cette barrière n’arrive pas, il faut conserver l’unité en
`FailedUnquiesced`, tuer un worker process isolé ou appliquer une politique process-wide. Continuer
comme si la recovery avait réussi n’est pas une option sûre.

Cette règle rejoint l’« intolerance vertueuse » des protocoles maintenus : les conditions
aberrantes doivent avoir une réaction spécifiée, plutôt qu’être silencieusement acceptées. Le
[RFC 9413](https://www.rfc-editor.org/rfc/rfc9413.html) souligne en même temps que l’erreur handling
doit préserver les traitements sans rapport. Pour ArmoniK, cela impose deux niveaux : un message
réseau invalide ferme l’operation ou la connection qui l’a reçu ; un `operation_id` impossible,
produit à l’intérieur du protocole d’isolation, met le runtime concerné en quarantaine.

## 4. Modèles d’exécution des langages cibles

**Résumé.** Les langages présentent des abstractions similaires mais leurs continuation semantics,
thread-affinity et règles d’ownership diffèrent. Une ABI qui suppose implicitement le modèle de Rust
exporte de la complexité vers tous les autres bindings.

**Intention.** Le message principal est que l’API projetée peut être idiomatique dans chaque
langage, alors que le protocole d’isolation reste commun. Les garanties ne doivent pas dépendre
d’une propriété propre à un seul scheduler sans qu’elle soit exprimée comme assumption résiduelle.

### 4.1 Rust et Tokio

**Résumé.** Une Rust `Future` est une state machine pollée coopérativement. Tokio schedule des tasks
réveillées sur un runtime qui peut être multi-threaded. Une task ne doit pas bloquer un worker et
doit rendre le contrôle à chaque `Pending` ou point d’await pertinent.

`Future::poll` ne promet pas un thread stable. `tokio::spawn` impose normalement `Send + 'static` et
une task peut migrer entre workers [Tokio, *Spawning*](https://tokio.rs/tokio/tutorial/spawning).
`select!` multiplexe plusieurs branches dans une même task ; elles ne s’exécutent pas simultanément,
mais leur ordre de sélection peut être nondeterministic sauf biais explicitement choisi [Tokio,
*Select*](https://tokio.rs/tokio/tutorial/select).

`spawn_blocking` confie une closure bornée à un pool distinct. Lorsque la limite de threads est
atteinte, les travaux supplémentaires attendent dans une queue. Une closure commencée ne peut plus
être abort ; le shutdown attend sa terminaison, ou cesse seulement d’attendre après un timeout sans
l’arrêter [Tokio, `spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).

Assumptions du modèle : Tokio et l’OS accordent finalement du CPU à une task réveillée ; toute
closure foreign dont la terminaison est nécessaire au progress retourne ; les ressources finies
nécessaires sont disponibles ou leur absence produit une erreur terminale. En face, le code Rust
doit garantir l’enregistrement du bon waker, l’absence de blocking étranger sur un worker async et
la publication d’une barrière de drain.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| future/task Rust | enregistre et réveille le bon waker ; ne bloque pas entre deux yields | driver I/O produit la readiness ; Tokio repoll la task |
| runtime FFI Rust | n’appelle que le trampoline autorisé et n’expose pas de lock au callback | le trampoline du binding retourne selon son contrat |
| Tokio | un wake rend la task éligible selon son scheduler ; `spawn_blocking` isole le worker async | OS/worker obtient du CPU ; les futures coopèrent en rendant la main |
| code foreign | si inclus dans la trusted base, retourne et n’exécute pas de user code ; sinon aucune garantie | ressources host disponibles ; runtime non finalisé |

### 4.2 .NET et C#

**Résumé.** TAP représente une opération par un `Task` actif ; `await` enregistre une continuation et
suspend la state machine C#. L’endroit où la continuation s’exécute dépend du `SynchronizationContext`,
du `TaskScheduler` et de la manière dont la completion est publiée.

TAP est le modèle recommandé depuis .NET Framework 4 ; APM (`Begin`/`End`) et EAP sont historiques
[Microsoft, *Asynchronous programming patterns*](https://learn.microsoft.com/en-us/dotnet/standard/asynchronous-programming-patterns/).
Une FFI complète généralement un `TaskCompletionSource`. Sans
`RunContinuationsAsynchronously`, une continuation peut s’exécuter inline sur le thread qui publie
la completion. Le spike utilise correctement cette option afin de ne pas exécuter indirectement du
code utilisateur sur un thread Tokio.

Le binding doit également maintenir vivant tout delegate stocké après un P/Invoke et le `GCHandle`
servant de contexte jusqu’au dernier callback [Microsoft, *Native interoperability best
practices*](https://learn.microsoft.com/en-us/dotnet/standard/native-interop/best-practices).
Le projet cible `netstandard2.0` et doit rester compatible avec .NET Framework ; `LibraryImport` et
les function pointers modernes ne peuvent donc pas être la seule baseline.

Assumptions du modèle : le runtime .NET schedule finalement le dispatcher et les continuations
postées ; l’application n’effectue pas de sync-over-async cyclique et ses handlers reviennent. Le
binding doit, lui, garantir que chaque `Task` est complété une fois, que les continuations
applicatives ne sont pas exécutées inline sur le thread natif, et que les roots survivent jusqu’à
la barrière terminale.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| binding .NET | outcome natif → bonne TCS exactement une fois ; roots conservées ; aucune exception à travers P/Invoke | event natif conforme ; runtime .NET opérationnel |
| projection transport | `CancellationToken` → demande native ; continuation utilisateur non inline sur le trampoline | binding accepte la commande ; scheduler accepte le post |
| BCL/runtime .NET | root GC respectée ; travail posté devient éligible | OS lui accorde du CPU ; aucune garantie de terminaison du handler |
| application | n’accède qu’à l’API safe ; une exception ordinaire reste hors du frame P/Invoke | projection respecte son contrat public ; son retour n’est supposé que par la liveness applicative |

### 4.3 Java 17, `CompletableFuture`, `Flow` et JNI

**Résumé.** Java 17 fournit `CompletableFuture` pour les valeurs terminales et `Flow` pour les
streams avec demand explicite. JNI reste la baseline native du dépôt. Le FFM API stabilisé en Java
22 améliore le lowering, mais ne peut pas être requis pour un artefact Java 17.

Une action non-async attachée à un `CompletableFuture` peut être exécutée par le thread qui complète
la future ; les variantes async utilisent un `Executor`, par défaut le common `ForkJoinPool`
[Java 17, `CompletableFuture`](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/util/concurrent/CompletableFuture.html).
`Flow.Subscription.request(n)` formalise un demand-based backpressure en messages one-way [Java 17,
`Flow`](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/util/concurrent/Flow.html).

Un native thread qui appelle Java par JNI doit être attaché à la JVM, utiliser un `JNIEnv` local au
thread et conserver les objets au moyen de global references. Les pending exceptions contraignent
les appels JNI suivants. Ces obligations rendent un callback depuis n’importe quel worker natif
plus coûteux conceptuellement qu’une boucle Java qui effectue des downcalls.

Le FFM API est final depuis Java 22 et propose downcalls, upcalls et arenas
[JEP 454](https://openjdk.org/jeps/454). Un upcall stub reste lié à la lifetime de son `Arena`, et
une exception échappant à sa target termine abruptement la JVM [Java 22,
`Linker`](https://docs.oracle.com/en/java/javase/22/docs/api/java.base/java/lang/foreign/Linker.html).
FFM réduit la plomberie JNI ; il ne supprime donc pas le contrat temporel.

Assumptions du modèle : l’`Executor` accepte et exécute finalement le travail, et les handlers de
l’application reviennent. Le binding ne peut pas reporter sur l’application l’attach/detach JNI,
la corrélation ou les global references : ce sont ses garanties envers la projection fonctionnelle.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| binding Java | corrélation vers la bonne future ; attach/detach ; global references ; exception JNI traitée | event natif conforme ; JVM accepte les opérations JNI légales |
| projection Java | `Flow.request(n)` devient une demand bornée ; dependent stage jamais exécuté dans le trampoline | binding accepte la command ; executor accepte le post |
| Executor/JVM | travail accepté devient éligible ; roots JNI sont respectées | OS schedule la JVM |
| application | demande, annule ou ferme selon le protocole `Flow` | projection respecte Reactive Streams |

### 4.4 Python et CPython

**Résumé.** `asyncio` exécute coopérativement une task à la fois par event loop. Un callback natif ne
doit pas exécuter arbitrairement du Python sur un foreign thread ; il doit s’attacher au bon
interpreter, acquérir les droits requis, puis généralement poster vers l’event loop.

Une `asyncio.Task` progresse jusqu’à son prochain await ; pendant l’attente d’une future, l’event
loop exécute d’autres tasks et callbacks [Python, `asyncio` tasks](https://docs.python.org/3/library/asyncio-task.html).
Sur une build CPython avec GIL, seul un thread possédant un attached thread state peut manipuler des
objets Python. Les threads créés nativement doivent être enregistrés ; la finalization et les
subinterpreters rendent le raccourci `PyGILState_Ensure` délicat [Python C API, *Thread states and
the GIL*](https://docs.python.org/3/c-api/threads.html).

Même une free-threaded build ne rend pas la FFI sans contrainte : l’attached thread state reste
nécessaire, la GC peut suspendre les threads et la lifetime de l’interpreter doit être respectée.

Assumptions du modèle : l’event loop désigné reste vivant, est pumpé et obtient du CPU ; les
coroutines applicatives reviennent à la loop. Le binding doit garantir la publication thread-safe,
l’attached thread state correct, la traduction de cancellation et l’absence d’entrée dans un
interpreter après sa barrière de finalization.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| binding Python | bon thread state ; post thread-safe ; cancellation native ; zéro entrée après sa barrière | interpreter et loop encore ouverts ; event natif conforme |
| event loop | callback accepté devient éligible dans la loop | application continue à pomper ; OS schedule le thread |
| application | demande, annule ou ferme le stream | projection respecte l’API `asyncio` |

### 4.5 C++11

**Résumé.** C++11 offre threads, condition variables, `std::future` et
`std::promise`, mais pas de continuation standard ni de stream asynchrone composable. Les APIs
performantes reposent donc souvent sur callbacks, event loops ou bibliothèques propres.

`std::future::get` est essentiellement une attente terminale. Une FFI peut fournir un callback C
avec `void* ctx`, ou un handle pollable enveloppé par RAII. La bibliothèque de binding doit choisir
un executor externe si elle veut offrir des continuations non bloquantes ; ce choix ne peut pas être
déduit du standard.

Assumptions du modèle : l’executor choisi est pumpé et les callables applicatifs reviennent. Le
wrapper C++11 doit garantir par RAII que `ctx`, callables et handles natifs survivent aux callbacks,
que les exceptions sont capturées avant le frame C et que le backpressure est traduit en commandes
explicites.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| wrapper C++11 | RAII après quiescence ; exception contenue ; demand traduite en crédit | ABI conforme ; executor accepte le post |
| executor choisi | callback accepté devient éligible | application/OS le pompe et lui donne du CPU |
| application | callable et executor respectent leur lifecycle public | wrapper respecte RAII et ordering |

### 4.6 C++20 et coroutines

**Résumé.** C++20 fournit le mécanisme de coroutine stackless, pas un runtime async ni un type
`task<T>` universel. L’awaiter ou la bibliothèque doit toujours décider où et quand reprendre la
coroutine.

Une coroutine conserve son état dans un coroutine frame et peut être reprise via un
`coroutine_handle`. `await_suspend` publie typiquement ce handle vers un scheduler ; la coroutine
peut être reprise sur un autre thread avant même le retour de `await_suspend`, ce qui impose une
synchronisation et interdit de toucher un awaiter potentiellement détruit après publication
[C++20 coroutines](https://en.cppreference.com/w/cpp/language/coroutines).

Une completion queue se mappe naturellement à un awaiter : elle stores le `coroutine_handle` sous
un token puis le dispatcher le reschedule. Un callback direct peut faire de même, mais doit choisir
si la reprise est inline ou postée vers un executor. C++20 ne tranche pas cette politique.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| wrapper/awaiter C++20 | publication sans race ; `stop_token` traduit ; aucune reprise inline non promise | type `task` et executor respectent enqueue/lifetime |
| type `task`/executor | frame conservé jusqu’au terminal ; coroutine ready rendue éligible | application/OS pompe et schedule l’executor |
| application | ne détruit pas illicitement une task encore owned | wrapper respecte le contrat d’awaiter |

## 5. Familles d’ABI asynchrones

**Résumé.** Les familles suivantes ne changent pas l’API fonctionnelle visée. Elles déterminent qui
détient l’état d’attente, qui choisit le thread de notification et qui doit assurer le drain.

**Intention.** Ce chapitre compare les unités d’ownership et de progress, non l’apparence de l’API
finale. Le choix doit minimiser les assumptions laissées au code applicatif tout en conservant des
garanties testables de backpressure, de cancellation et de shutdown.

### 5.1 Callback direct

**Résumé.** La bibliothèque native appelle une function pointer fournie par le consumer. Le chemin
critique est court et la bibliothèque peut éviter une queue explicite, mais l’exécution foreign
entre au cœur de son scheduler et de son lifecycle.

```c
typedef void (*on_event_fn)(void *ctx, const event_t *event);

status_t operation_start(runtime_t *, on_event_fn, void *ctx, operation_t **out);
```

MsQuic est l’exemple le plus abouti de cette famille. Il autorise certains downcalls depuis le
callback, garantit par défaut l’absence de callback récursif et fait de `SHUTDOWN_COMPLETE` le point
sûr de destruction. Cette simplicité apparente repose sur une attention spécifique aux cycles de
calls et à l’ordre interne [MsQuic, *Execution*](https://microsoft.github.io/msquic/msquicdocs/docs/Execution.html).

Les variantes sont importantes :

- callback inline sur le protocol thread : latency minimale, mais un callback lent retarde le
  protocole ;
- callback via `spawn_blocking` attendu : protège les workers, conserve l’ordre, mais la request
  dépend du retour du callback ;
- callback via `spawn_blocking` détaché : le transport progresse, mais ordre, ownership et shutdown
  exigent une sérialisation séparée ;
- callback via un dispatcher thread dédié : modèle push contrôlé, proche d’une completion queue
  interne.

Un `spawn_blocking` par event n’est pas une primitive de cancellation. Une task commencée ne peut
pas être abort et peut prolonger le shutdown. Si le callback fait un sync-over-async sur une action
qui exige la reprise de la request après le callback, le deadlock est immédiat.

```text
request task attend callback
callback arme Read puis attend ReadDone
request task ne peut produire ReadDone avant le retour du callback
```

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| runtime natif | ordering promis ; aucun lock exposé ; liste de downcalls reentrant ; zéro nouveau callback après barrière | trampoline conforme et retournant |
| trampoline du binding | `ctx` rooted ; validation/copie/retain ; exception contenue ; aucun user code ; aucun wait protocolaire avant retour | runtime du langage utilisable ; thread obtient du CPU ; allocation finie retourne succès ou erreur |
| projection/app | continuation postée hors callback ; aucune connaissance du thread natif | scheduler applicatif et handler progressent |

Dans une variante MsQuic-style avec binding officiel, `TrampolineReturns` n’est donc plus une
assumption laissée à l’application : c’est une garantie du runtime de binding. Elle reste une
assumption du runtime natif, mais la composition doit la décharger par cette garantie vérifiée.

### 5.2 Future handle

**Résumé.** L’appel retourne un opaque handle. Le consumer le poll, enregistre un waker/callback ou
attend sa readiness, puis extrait le résultat et libère le handle.

```c
future_t *operation_start(...);
status_t future_poll(future_t *, wake_fn, uint64_t wake_data);
status_t future_complete(future_t *, result_t *out);
void future_cancel(future_t *);
void future_free(future_t *);
```

FoundationDB garantit qu’un callback de future est invoqué au plus une fois, mais il peut être
appelé inline pendant son enregistrement si la future est déjà ready, ou ultérieurement sur le
network thread. La documentation interdit d’y bloquer le network thread et lie la lifetime des
résultats à celle du future handle [FoundationDB C API](https://apple.github.io/foundationdb/api-c.html).

UniFFI encapsule une Rust `Future` dans un handle avec quatre opérations : poll, complete, cancel et
free. Le callback de poll peut être immédiat, ce qui est précisément motivé par la coordination de
lifetime du callback data [UniFFI, *Async overview*](https://mozilla.github.io/uniffi-rs/latest/internals/async-overview.html).

Ce modèle convient bien à une fonction `async fn -> T`. Pour un transport duplex, chaque read et
write peut devenir une future, mais une machine englobante reste nécessaire pour headers, trailers,
half-close et terminal outcome.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| future native | readiness monotone ; wake du callback enregistré ; complete/cancel/free contractuels | callback data live ; calcul producteur progresse |
| binding | state publié avant poll ; repoll après wake ; extraction/free exactement une fois | runtime cible schedule le repoll |
| projection duplex | cohérence entre les différentes futures d’une call | demand, peer/timeout et binding progressent |

### 5.3 Completion queue et waitable set

**Résumé.** Le code natif publie des events possédés dans une queue. Le binding attend ou poll un
lot d’events, les route vers des opérations locales, puis libère chaque event.

```c
status_t runtime_wait(runtime_t *, event_t *events, size_t capacity,
                      duration_t timeout, size_t *count);
void event_release(runtime_t *, event_t *);
```

gRPC Core utilise des completion queues et des tags. Le shutdown n’est complet qu’après demande
d’arrêt et drain de tous les tags ; la queue ne peut être détruite qu’ensuite [gRPC,
`CompletionQueue::Shutdown`](https://grpc.github.io/grpc/cpp/classgrpc_1_1_completion_queue.html).
L’ancien binding C# utilisait des polling threads managés et un registry pour projeter ces tags en
tasks.

Le Component Model async de WASI 0.3 généralise futures, streams, subtasks et waitable sets. Les
waitable sets sont explicitement comparés à un `epoll` simplifié ; si plusieurs events sont ready,
leur choix est nondeterministic [Component Model, *Concurrency
Explainer*](https://github.com/WebAssembly/component-model/blob/main/design/mvp/Concurrency.md).

Cette famille impose une queue capacity et une stratégie de wakeup explicites. En échange, le
binding contrôle son thread, son executor et son contexte de runtime. Les payloads possédés peuvent
rester zero-copy côté Rust jusqu’à `event_release`; la projection managée peut néanmoins nécessiter
une copie selon son API.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| runtime natif | publication atomique event+owner ; queue bornée sans perte ; barrière native fidèle | binding retire des events ou déclenche shutdown |
| runtime de binding | routing/quarantaine ; release exactly-once ; drain jusqu’au terminal | dispatcher schedulé ; events conformes |
| projection/app | transformation en future/stream sans voir la queue | binding progresse ; consumer demande/annule/ferme |

### 5.4 Event loop natif piloté par le langage cible

**Résumé.** Une variante de la completion queue intègre le handle natif à l’event loop du langage :
file descriptor, eventfd, Windows event ou appel `wait` effectué par un EventLoop thread.

Netty illustre la logique générale : les transports epoll/kqueue sont natifs, mais l’EventLoop Java
reste propriétaire du scheduling de la pipeline. L’implémentation JNI peut réduire allocations et
overhead tout en conservant la projection Netty [Netty, *Native
transports*](https://netty.io/wiki/native-transports.html).

Cette approche évite un thread supplémentaire si le runtime cible sait enregistrer le waitable.
Elle augmente cependant la matrice platform × language et risque de rendre le noyau moins
portable. Un premier design peut fournir `runtime_wait` portable, puis ajouter un native wait
handle comme capability optionnelle.

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| runtime natif | signal edge/level sans lost wakeup ; fallback `wait` défini | primitive OS conforme |
| binding | registration/deregistration emboîtées avec le lifecycle natif | barrière native reçue ; event loop accepte le handle |
| event loop | waitable ready rendu éligible | application continue à la pomper ; OS la schedule |

### 5.5 Stackful et stackless lowering

**Résumé.** WASI montre qu’une même interface `async func`, `future<T>` ou `stream<T>` peut être
lowered en mode stackful ou stackless. Cette distinction est utile conceptuellement, mais importer
le Canonical ABI complet dans une DLL native créerait un runtime disproportionné.

Dans le stackful ABI du Component Model, une invocation peut suspendre son stack. Dans le stackless
callback ABI, l’export rend explicitement `EXIT`, `YIELD` ou `WAIT(set)` puis le runtime appelle une
continuation lorsque l’exécution peut reprendre. Le callback intervient donc après un retour
explicite à l’event loop, et non comme un upcall arbitraire au milieu d’un downcall.

La leçon transférable n’est pas la signature Core Wasm. C’est la séparation entre types
fonctionnels, waitables et lowering propre au runtime. Pour ArmoniK, un waitable native minimal peut
s’en inspirer sans reproduire stack switching, component instance tables et Canonical ABI.

### 5.6 API bidirectionnelle : notifications et host calls

**Résumé.** « Rust appelle C# » peut signifier une notification sans réponse ou une invocation dont
Rust attend le résultat. Ces deux cas doivent être distincts ; aucun n’exige nécessairement un
function pointer managé.

Un log record peut être un event runtime :

```text
Rust -- LOG_RECORD --> dispatcher -- ILogger.Log --> application
```

Le transport ne doit pas dépendre de son traitement. Une queue de diagnostics séparée ou une
politique de drop explicite évite qu’un flot `TRACE` bloque les completions fonctionnelles.

Un token provider est un host call corrélé :

```text
Rust -- HOST_CALL(id, GetToken) --> binding
Rust <-- HOST_REPLY(id, token|error) -- binding
```

Seule l’opération dépendante est suspendue. Cancellation et shutdown doivent résoudre tous les host
calls pending. UniFFI représente également les async callback interfaces par une function de
completion exactement-once [UniFFI, *Async FFI details*](https://mozilla.github.io/uniffi-rs/next/internals/async-ffi.html).

| Point de vue | Garantie contrôlée | Assumption externe |
|---|---|---|
| runtime natif | log lossy séparé du progress ; invocation pending résolue par reply/fail/cancel/shutdown | broker livre les requests et replies |
| broker du binding | corrélation exactement une fois ; quotas diagnostics ; résolution locale sur faute | dispatcher schedulé ; service host accepte l’invocation |
| service host | toute invocation acceptée reçoit reply ou erreur | handler utilisateur retourne ou deadline locale expire |

## 6. Études de cas et enseignements transférables

**Résumé.** Les systèmes aboutis n’éliminent pas les obligations ; ils les concentrent dans une
couche et les rendent explicites. Les points les plus transférables sont le terminal event de
shutdown, l’ownership lié au handle, le demand-based flow control et la séparation entre
notification native et continuation utilisateur.

**Intention.** Le lecteur doit retenir les mécanismes reproductibles et leurs assumptions, pas
copier l’API d’un projet. Un exemple industriel démontre une faisabilité dans son environnement ;
il ne prouve ni son optimalité pour ArmoniK ni la composition avec nos runtimes cibles.

### 6.1 WASI 0.3 et le Component Model

WASI 0.3, ratifié le 11 juin 2026, introduit nativement `async func`, `future<T>` et `stream<T>`
[WASI 0.3 release notes](https://github.com/WebAssembly/WASI/releases/tag/v0.3.0). Les readable et
writable ends ont une ownership unique ; subtasks, futures et streams sont waitables ; cancellation
reste coopérative et peut perdre la race contre un return.

Ce modèle est une référence forte pour l’API fonctionnelle et les waitables. Ses limites de
transposition sont nettes : le Canonical ABI cible Core Wasm, dispose d’un component store, de
tables, de task bookkeeping et éventuellement de stack switching. `wit-bindgen` génère surtout des
guests WebAssembly, pas les bindings d’une DLL C native ; le support Java historique a été retiré
[wit-bindgen](https://github.com/bytecodealliance/wit-bindgen).

### 6.2 FoundationDB

FoundationDB expose une C API stable à opaque futures. Le résultat emprunté reste valide jusqu’à
`future_destroy` ou `future_release_memory`. Le callback peut être immédiat ou différé sur le
network thread. Son binding Java publie le state avant d’enregistrer le callback rapide, passe par
un `Executor` pour le unmarshalling et coordonne le native pointer sous lock.

L’enseignement est double : une future handle peut être très performante, mais la « fast
completion race » est une règle fondamentale du contrat ; le binding officiel reste une vraie
couche protocolaire, pas seulement du code JNI généré.

### 6.3 gRPC Core et son ancien binding C#

La completion queue rend les events explicites et donne au binding le choix des polling threads.
Les tags corrèlent les operations ; le shutdown suit la séquence request, drain, destroy [gRPC Core,
*Completion Queue*](https://grpc.github.io/grpc/core/md_doc_core_grpc-cq.html).

Le coût est une registry, des polling threads et un hop vers le ThreadPool. Le bénéfice est une
frontière claire : Core ne rentre pas arbitrairement dans une continuation C#. Cette architecture
constitue une preuve d’industrialisation de la famille, pas une preuve qu’elle est optimale pour les
tailles d’events ArmoniK.

### 6.4 MsQuic et System.Net.Quic

MsQuic choisit les callbacks directs pour éviter l’état et les calls nécessaires au modèle socket.
Il exige des handlers courts et traite explicitement les reentrant downcalls pour éviter les
deadlocks. Le receive flow control restitue l’ownership du buffer par
`StreamReceiveComplete`; le shutdown complete autorise enfin la destruction [MsQuic,
*Using Streams*](https://microsoft.github.io/msquic/msquicdocs/docs/Streams.html).

Ce modèle montre que le callback direct n’est pas intrinsèquement incorrect. Il montre aussi le
niveau de contrat et d’engineering requis pour le rendre composable. Son économie de queue native
est payée par un protocole callback précis et par une adaptation non triviale dans System.Net.Quic.

### 6.5 GIO `GTask`

`GTask` capture le thread-default `GMainContext` au démarrage et y invoque le callback, au plus tôt à
une prochaine itération lorsque nécessaire. Toute task créée avec callback doit être complétée par
un `g_task_return_*`; le main context doit continuer à tourner jusqu’à completion
[GIO `Task`](https://docs.gtk.org/gio/class.Task.html).

Il s’agit d’un exemple important de callback contrôlé : la bibliothèque n’appelle pas le consumer
depuis n’importe quel worker, elle capture son scheduler. Une FFI multi-langage ne peut pas supposer
qu’un équivalent commun existe, mais son binding peut reproduire ce pattern via un dispatcher.

### 6.6 UniFFI

UniFFI laisse le runtime foreign piloter la future Rust. C’est une bonne factorisation pour les
fonctions async et une source d’exemples de lifetime/cancellation. Le modèle de poll contient encore
un callback de wake ; le streaming complexe demande des ressources supplémentaires et une machine
de domaine.

### 6.7 Netty

Netty montre qu’un binding Java performant peut garder l’EventLoop du côté Java et employer des
downcalls JNI vers epoll/kqueue. Cela réduit les upcalls arbitraires et facilite la thread-affinity,
au prix d’un code natif platform-specific. L’enseignement transférable est le contrôle du scheduler
par la projection Java, non l’adoption obligatoire d’epoll dans l’ABI.

### 6.8 Modèles académiques de systèmes asynchrones

Les modèles académiques plus généraux éclairent le choix de l’unité de composition, mais chacun
abstrait une partie décisive de la FFI réelle.

Les **Kahn Process Networks** composent des processus déterministes qui communiquent par channels
FIFO conceptuellement non bornés ; les reads sont bloquantes et les writes ne le sont pas. Cette
discipline donne une sémantique déterministe utile pour séparer calcul fonctionnel et transport
[Kahn, 1974](https://perso.ensta.fr/~chapoutot/various/kahn_networks.pdf). La transposition directe
échoue dès qu’une queue FFI est bornée, qu’un producer doit subir du backpressure, que cancellation
supprime un message ou qu’une sélection entre plusieurs inputs dépend du scheduler. Ces choix ne
sont pas des détails d’implémentation : ils changent les traces.

Le **Actor model** met l’accent sur l’envoi asynchrone, les mailboxes, l’isolation d’état et la
création dynamique [Agha, 1986](https://mitpress.mit.edu/9780262511414/actors/). Un runtime et ses
operations se modélisent naturellement comme actors. Le modèle général ne fournit cependant pas,
à lui seul, une garantie de livraison, d’ordering entre senders, de fairness de mailbox, de
backpressure ou de recovery. Ces propriétés appartiennent à la sémantique de l’actor system choisi.

Les **Communicating Sequential Processes** placent la communication et la composition parallèle au
centre de la spécification [Hoare, 1978](https://doi.org/10.1145/359576.359585). Les traces, refus
et deadlocks donnent un excellent langage pour décrire ce qu’un endpoint peut accepter. Le rendez-
vous synchrone du modèle initial n’est toutefois pas une completion queue asynchrone : introduire
des buffers, leur capacité et la fairness de leur service produit un autre système. Les session
types de la section 3.2 apportent une projection typée, mais conservent la même nécessité de rendre
explicites transport, scheduler et failure model.

Ces trois familles convergent vers une même conclusion pratique : la state machine fonctionnelle,
la file concrète et le scheduler doivent être des objets distincts du modèle. Les confondre ferait
hériter la preuve d’assumptions — queue infinie, rendez-vous équitable ou mailbox toujours servie —
qui ne sont garanties par aucun runtime cible.

### 6.9 MsQuic et WASI ne résolvent pas le même problème

WASI part d’une interface de composant et définit plusieurs lowerings async, des resources et des
waitable sets dans un runtime qui possède store, tables et task bookkeeping. C’est une référence
forte pour séparer l’interface fonctionnelle de son lowering et pour raisonner sur futures/streams.
Importer sa forme waitable dans une DLL native recrée toutefois une partie de ce runtime.

MsQuic part au contraire d’une ABI C concrète et performante. Le native runtime choisit le thread,
sérialise les callbacks selon son contrat et utilise une shutdown-complete barrier ; le callback
doit rendre rapidement le contrôle. Ce modèle devient composable avec un managed host lorsque le
callback n’est pas l’application mais un trampoline du binding :

```text
Rust FFI assume TrampolineReturns
Host FFI guarantees TrampolineReturns
Host FFI assume ExecutorEventuallyRuns
Host runtime/OS provides the remaining fairness
```

La deuxième ligne est la différence décisive avec un callback arbitraire. Le trampoline valide le
token, copie ou retain le payload, effectue une transition bornée, contient l’exception et poste la
continuation ; il ne lance aucun handler utilisateur. La liveness du reactor natif ne dépend donc
plus de `UserHandlerReturns`.

Cette simplification a ses limites. Une pause GC/JVM suspendant le trampoline suspend aussi le
worker natif ; JNI et Python rendent l’upcall plus délicat ; les callbacks non armés comme logs et
faults exigent leur propre bound. Si ces coûts empêchent le binding de garantir son contrat, le
waitable reprend l’avantage. L’état de l’art ne donne donc pas « callbacks contre WASI » comme choix
abstrait : il propose de comparer deux compositions complètes, chacune avec ses assumptions
résiduelles.

La garantie est en outre language-specific. En .NET, une `TaskCompletionSource` créée avec
`RunContinuationsAsynchronously` permet de publier la completion sans lancer la continuation dans
l’upcall. En Java, appeler directement `CompletableFuture.complete` ne suffit pas : un dependent
stage non-async peut s’exécuter sur le thread qui complète. Le trampoline doit alors seulement
publier l’état et poster vers un mécanisme dont l’exécution non-inline est garantie. MsQuic fournit
donc une forme de protocole native ; il ne fournit pas automatiquement le trampoline correct de
chaque managed runtime.

## 7. Factorisation et génération de bindings

**Résumé.** Les déclarations et codecs se génèrent bien ; les protocoles temporels se génèrent
seulement si leur state machine fait partie de la source. La meilleure unité de factorisation est
un petit runtime d’isolation commun, réimplémenté idiomatiquement par langage, plus des projections
de domaine.

**Intention.** Le message principal est de partager la spécification, les traces et les tests avant
de chercher à partager tout le code. La première implémentation peut rester dans le package
fonctionnel ; l’extraction d’un runtime FFI générique devra être justifiée par une seconde FFI
réelle.

Quatre couches doivent rester visibles :

| Couche | Factorisation attendue | Contenu |
|---|---:|---|
| ABI brute | élevée | symboles, calling convention, structs, constants |
| Runtime de protocole FFI | élevée dans un langage | dispatcher, handles, events, cancel, shutdown |
| Adaptateur de domaine | moyenne | mapping event types, codecs, state machine transport |
| API fonctionnelle | faible entre domaines | `Transport`, `Call`, streams, erreurs idiomatiques |

Un schéma peut générer :

- headers C et declarations P/Invoke/JNI/FFM ;
- enum values, struct layouts et capability tables ;
- codecs de records et erreurs ;
- squelettes d’event routing ;
- tests de conformance de layouts et traces.

Il ne peut pas déduire d’une signature :

- si un callback peut précéder le retour de `start` ;
- si reentrancy est permise ;
- quand un payload cesse d’être valide ;
- quelle fairness rend `Completed` inévitable ;
- comment le runtime est drainé avant unload.

SWIG apporte davantage que les signatures et les directors : ses typemaps savent projeter des
exceptions synchrones à travers une frontière C#/P/Invoke ou Java/JNI. Le point subtil est qu’une
exception foreign ne traverse pas réellement la stack native. Le wrapper intercepte l’exception
C++ ou le status C, installe une exception pending côté cible, retourne immédiatement du wrapper
natif, puis le stub cible la lève après le retour du downcall.

En C#, SWIG utilise une reverse delegate pour mémoriser l’exception pending. Un typemap qui peut
lever doit être marqué `canthrow` ; le wrapper unmanaged doit cesser son travail immédiatement, et
une seconde exception pending ne doit pas écraser la première
([SWIG C# — Exception handling](https://www.swig.org/Doc4.3/CSharp.html)). En Java, `%exception`,
`%javaexception`, `%catches` et les typemaps `throws` organisent la traduction vers une exception
JNI pending ; les appels JNI autorisés sont ensuite restreints jusqu’à son observation ou son
effacement ([SWIG Java — Exception handling](https://www.swig.org/Doc4.3/Java.html)). Les directors
peuvent aussi remapper une exception Java vers C++, mais uniquement avec un contrat de typemap et
de catch explicite.

C’est un vrai avantage pour les fonctions synchrones : le code généré concentre une mécanique
facile à rater. Cela ne rend pas légal l’unwind à travers C et ne résout pas les opérations async.
Une erreur survenant après le retour du downcall doit devenir l’état terminal d’une future ou un
event possédé ; elle ne peut plus être portée par la stack d’initiation. SWIG ne déduit pas non plus
le thread d’upcall, le rooting jusqu’à completion, l’exactly-once, le backpressure ou le shutdown.

`cbindgen` et `csbindgen`, déjà employés par le dépôt, restent adaptés à l’ABI brute si la state
machine et les tests demeurent des artefacts de premier rang. SWIG devient une option intéressante
si la valeur de ses typemaps synchrones et directors dépasse le coût d’introduire un second système
de génération ; cette décision est distincte de celle du protocole async.

Une factorisation réaliste à terme serait :

```text
armonik-transport::runtime (première implémentation Rust)
├── runtime / operation / event / payload
├── wait, wakeup, cancel, release, shutdown
└── capabilities

Runtime interne au package fonctionnel .NET
Runtime interne au package fonctionnel Java
Runtime interne au package fonctionnel Python
Runtime interne au wrapper fonctionnel C++

armonik-transport adapters
└── headers, duplex body, trailers, transport errors
```

Le code n’est pas partagé entre runtimes managés, mais le schéma, le modèle TLA+, les traces et la
test suite le sont. Après une seconde FFI réelle, les parties démontrées génériques pourront être
extraites en `armonik-ffi-runtime` et en packages host séparés. Les extraire dès le premier cas
cristalliserait probablement des concepts propres au transport sous des noms faussement généraux.

## 8. Limites techniques et scientifiques de la comparaison

**Résumé.** Aucun tableau ne permet de choisir sans workload. Les coûts de queue, callback, copy et
scheduler hop dépendent des payloads, du débit, du nombre de requests, du runtime cible et de la
politique de backpressure. Les garanties de liveness dépendent de composants hors contrôle.

**Intention.** Le lecteur doit retenir qu’une preuve fonctionnelle et une mesure de performance
répondent à deux questions différentes. Les deux sont nécessaires : formaliser ne remplace pas les
benchmarks, et un benchmark nominal ne révèle pas un deadlock ou un use-after-free rare.

### 8.1 Absence de domination universelle

Un callback direct peut gagner sur de petits events fréquents en évitant dequeue et batching state.
Une completion queue peut gagner sous charge grâce au batching, à la cache locality et à une
réduction du nombre de transitions native/managed. Une copie peut être plus rapide qu’un protocole
de lifetime complexe pour de petits buffers, et catastrophique pour de grands streams.

Ces propositions sont des hypothèses de performance, pas des résultats. Elles doivent être mesurées
sur les versions et plateformes supportées.

### 8.2 Unbounded environment

Le nombre de requests, la taille des streams, la durée des callbacks et le silence du peer sont
potentiellement non bornés. TLC exige des bornes finies. Le design devra expliquer quelles bornes
sont abstraction-preserving et compléter le model checking par invariants, refinement et tests.

### 8.3 Cancellation n’est pas rollback

Dans tous les modèles étudiés, cancellation peut perdre la race contre completion et n’annule pas
nécessairement l’effet distant. Elle signifie d’abord « le consumer ne souhaite plus attendre » ou
« demander coopérativement l’arrêt ». La documentation doit séparer request, acknowledgement et
terminal outcome.

### 8.4 Scheduling hors du contrat de la bibliothèque

Une bibliothèque peut wake un executor ; elle ne peut garantir qu’un process non schedulé obtienne
du CPU. Elle peut poster sur un event loop ; elle ne peut garantir que l’application le pompe. Elle
peut demander à un callback de retourner ; elle ne peut pas l’y contraindre sans isolation plus
forte qu’une FFI in-process.

### 8.5 Memory safety partielle

Rust peut garantir l’ownership à l’intérieur de la bibliothèque. Dès qu’un raw pointer traverse
l’ABI, la conformité du consumer redevient une hypothèse. Les owned event handles réduisent la
fenêtre d’emprunt mais ne rendent pas impossible un double release dans du C. Les bindings safe
doivent être considérés comme une partie de la trusted computing base.

### 8.6 Limite cyber de l’isolation in-process

Une ABI C durcie réduit les erreurs accidentelles et l’exploitabilité d’inputs non fiables ; elle
ne crée pas une security boundary entre deux composants natifs du même process. La documentation
Python rappelle explicitement que `ctypes` contourne les mécanismes de safety et permet au code
natif de compromettre le process ; la spécification JNI formule le même risque pour la JVM
([Python](https://docs.python.org/3/library/ctypes.html),
[JNI](https://docs.oracle.com/en/java/javase/17/docs/specs/jni/intro.html)). La Rust Reference ajoute
qu’un undefined behavior d’un côté de la FFI affecte le programme entier
([Rust](https://doc.rust-lang.org/stable/reference/behavior-considered-undefined.html)).

Le threat model doit donc séparer :

- un peer réseau malveillant, dont les bytes restent des inputs non fiables et bornés ;
- une application utilisant mal un binding safe, qui doit recevoir une erreur structurée ;
- un binding buggé, contre lequel generation counters, registries et runtime checks apportent une
  défense en profondeur ;
- une bibliothèque native malveillante ou déjà compromise, contre laquelle l’in-process ne promet
  ni confidentiality ni integrity.

Si le dernier adversaire est dans le scope, la primitive pertinente est une isolation de process ou
un sandbox disposant d’un modèle de mémoire séparé, par exemple WebAssembly dont le security model
spécifie une sandbox et des host APIs contrôlées
([WebAssembly](https://webassembly.org/docs/security/)). Le coût d’IPC, de copie, de déploiement et
de recovery devient alors une dimension de design à mesurer. Une completion queue in-process
améliore le fault containment logique ; elle ne remplace pas cette security boundary.

Les checks doivent couvrir types, tailles, capacités et relations entre champs avant allocation ou
déréférencement. Cela correspond notamment aux familles
[CWE-20](https://cwe.mitre.org/data/definitions/20.html),
[CWE-1284](https://cwe.mitre.org/data/definitions/1284.html),
[CWE-672](https://cwe.mitre.org/data/definitions/672.html) et
[CWE-772](https://cwe.mitre.org/data/definitions/772.html). Les limites et quotas sont eux-mêmes une
propriété de sécurité : une registry de tombstones, une queue de diagnostics ou une boucle de
restart non bornée déplace seulement le denial of service
([CWE-400](https://cwe.mitre.org/data/definitions/400.html)).

## 9. Critères qui doivent guider le choix ArmoniK

**Résumé.** Le choix doit maximiser la capacité à offrir une API idiomatique et un contrat
vérifiable dans plusieurs langages, sous une enveloppe de performance mesurée. Il ne doit pas être
réduit au nombre de fonctions C ou au microcoût d’un seul event.

**Intention.** Ces critères expliquent le choix MsQuic-style de la V1 et définissent les conditions
qui imposeraient de le rouvrir. Ils restent applicables aux refinements propres à chaque binding.

Les critères de décision sont :

1. safety vérifiable : ownership, exactly-once, ordering, no-callback-after-shutdown ;
2. liveness composable : aucune promesse sans assumption de fairness explicite et sans garantie
   voisine ou assumption résiduelle qui la décharge ;
3. compatibilité : .NET Framework/netstandard2.0 et Java 17 en baseline ;
4. coût par event : crossings, allocations, copies, wakeups, context switches ;
5. throughput sous streams concurrents et backpressure ;
6. complexité du premier binding et coût marginal du suivant ;
7. support des appels synchrones courts et des host calls bidirectionnels ;
8. capacité de versionner et négocier les capabilities dans les deux sens ;
9. shutdown déterministe dans un host long-lived ou unloadable ;
10. correspondance simple entre modèle TLA+, runtime checks et tests exécutables.
11. failure containment explicite : classification des fautes, quarantaine, barrière de quiescence,
    diagnostics bornés et politique de recovery ou de fail-fast.

Le [design](DESIGN.md) transforme ces critères en options ArmoniK et en plan de validation. La
[comparaison de baseline](BASELINE_COMPARISON.md) situe l’implémentation courante et le spike C# dans
ce paysage. Le [plan de remédiation](PR_REMEDIATION.md) rattache enfin chaque propriété à la PR où
elle doit être introduite.

## Références principales

**Résumé.** Cette liste regroupe les références utilisées comme fondations plutôt que comme simples
analogies. Les liens détaillés vers les documentations industrielles figurent dans les chapitres
correspondants.

**Intention.** Ces sources délimitent ce qui est emprunté à des résultats établis et ce qui demeure
une proposition de design à valider expérimentalement ou formellement pour ArmoniK.

- D. Brand, P. Zafiropulo, “On Communicating Finite-State Machines”, JACM 1983,
  [DOI 10.1145/322374.322380](https://doi.org/10.1145/322374.322380).
- G. Kahn, “The Semantics of a Simple Language for Parallel Programming”, IFIP 1974,
  [article](https://perso.ensta.fr/~chapoutot/various/kahn_networks.pdf).
- C. A. R. Hoare, “Communicating Sequential Processes”, CACM 1978,
  [DOI 10.1145/359576.359585](https://doi.org/10.1145/359576.359585).
- R. H. Halstead Jr., “Multilisp: A Language for Concurrent Symbolic Computation”, TOPLAS 1985,
  [DOI 10.1145/4472.4478](https://doi.org/10.1145/4472.4478).
- G. Agha, *Actors: A Model of Concurrent Computation in Distributed Systems*, MIT Press 1986,
  [MIT Press](https://mitpress.mit.edu/9780262511414/actors/).
- B. Liskov, L. Shrira, “Promises: Linguistic Support for Efficient Asynchronous Procedure Calls in
  Distributed Systems”, PLDI 1988, [DOI 10.1145/53990.54016](https://doi.org/10.1145/53990.54016).
- L. Lamport, “The Temporal Logic of Actions”, TOPLAS 1994,
  [DOI 10.1145/177492.177726](https://doi.org/10.1145/177492.177726).
- L. de Alfaro, T. A. Henzinger, “Interface Automata”, FSE 2001,
  [DOI 10.1145/503209.503226](https://doi.org/10.1145/503209.503226).
- M. Fähndrich et al., “Language Support for Fast and Reliable Message-based Communication in
  Singularity OS”, EuroSys 2006,
  [Microsoft Research](https://www.microsoft.com/en-us/research/publication/language-support-for-fast-and-reliable-message-based-communication-in-singularity-os/).
- Z. Stengel, T. Bultan, “Analyzing Singularity Channel Contracts”, ISSTA 2009,
  [DOI 10.1145/1572272.1572275](https://doi.org/10.1145/1572272.1572275).
- B. Lee et al., “Jinn: Synthesizing Dynamic Bug Detectors for Foreign Language Interfaces”, PLDI
  2010, [DOI 10.1145/1806596.1806601](https://doi.org/10.1145/1806596.1806601).
- G. Tan, “JNI Light: An Operational Model for the Core JNI”,
  [article](https://www.cse.psu.edu/~gxt29/papers/jnimodel.pdf).
- K. Honda, N. Yoshida, M. Carbone, “Multiparty Asynchronous Session Types”, JACM 2016,
  [DOI 10.1145/2827695](https://doi.org/10.1145/2827695).
- A. Radhakrishna et al., “DroidStar: Callback Typestates for Android Classes”, ICSE 2018,
  [Microsoft Research](https://www.microsoft.com/en-us/research/publication/droidstar-callback-typestates-for-android-classes/).
- S. Meier, S. Mover, B.-Y. E. Chang, “Lifestate: Event-Driven Protocols and Callback Control Flow”,
  ECOOP 2019, [DOI 10.4230/LIPIcs.ECOOP.2019.1](https://doi.org/10.4230/LIPIcs.ECOOP.2019.1).
- C. Newcombe et al., “How Amazon Web Services Uses Formal Methods”, CACM 2015,
  [DOI 10.1145/2699417](https://doi.org/10.1145/2699417).
- R. D. Schlichting, F. B. Schneider, “Fail-stop Processors: An Approach to Designing Fault-tolerant
  Computing Systems”, TOCS 1983,
  [DOI 10.1145/357369.357371](https://doi.org/10.1145/357369.357371).
- G. Candea et al., “Microreboot — A Technique for Cheap Recovery”, OSDI 2004,
  [USENIX](https://www.usenix.org/conference/osdi-04/microreboot%E2%80%94-technique-cheap-recovery).
- M. Thomson, D. Schinazi, “Maintaining Robust Protocols”, RFC 9413, 2023,
  [RFC Editor](https://www.rfc-editor.org/rfc/rfc9413.html).
- NIST, “Secure Software Development Framework (SSDF) Version 1.1”, SP 800-218, 2022,
  [DOI 10.6028/NIST.SP.800-218](https://doi.org/10.6028/NIST.SP.800-218).
