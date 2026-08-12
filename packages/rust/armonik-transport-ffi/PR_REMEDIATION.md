# Plan de remédiation de la stack de PR FFI

## Statut, périmètre et intention

**Résumé.** Ce document traduit la décision d’architecture MsQuic-style en un plan de travail sur
la stack GitHub ouverte au 10 août 2026. Il distingue ce qui peut être conservé, ce qui doit être
amendé dans une PR existante et ce qui doit être remplacé avant qu’une nouvelle couche ne dépende
d’un contrat incorrect. Il ne demande ni merge ni fermeture automatique.

**Intention.** Le lecteur doit retenir que la remédiation ne sera pas une PR de « hardening » posée
au sommet de la stack. Une règle de lifecycle, d’identity ou d’ownership est introduite dans la
première PR qui crée l’objet concerné, puis les PR suivantes sont rebased. Le code du reactor actuel
est une source à reprendre, pas une ABI à entériner telle quelle.

La décision de V1 est la suivante :

```text
armonik-transport::runtime
        |
        v
Rust FFI --callback C--> trampoline officiel du binding
                              |
                              +--> copie/retain du payload
                              +--> publication dans une concurrent queue host
                              +--> signal du scheduler host
                              +--> retour borné, sans user code
```

Le modèle waitable inspiré de WASI reste un état de l’art, un contre-modèle TLA+ et une solution de
repli si une cible ne peut pas fournir le trampoline. Il n’est pas un second prototype obligatoire
de V1. `spawn_blocking(callback).await` et le callback applicatif arbitraire ne font pas partie du
design retenu.

## 1. Règles de transformation de la stack

**Résumé.** Les PR sont ordonnées par responsabilité architecturale, pas seulement par leur base
Git actuelle. Une modification est placée là où apparaît pour la première fois l’invariant qu’elle
doit garantir.

**Intention.** L’objectif est qu’aucune PR intermédiaire ne publie sciemment un contrat que la PR
suivante devra contourner. Cela réduit le code de compatibilité temporaire, rend les reviews locales
et permet de relier chaque invariant TLA+ au diff qui l’implémente.

Les règles pratiques sont :

1. Un objet est créé avec son lifecycle complet : owner, children, fermeture du start gate,
   quiescence et condition de destruction.
2. Une identity routable est créée avec son scope et sa generation. Une adresse réutilisable ne
   constitue pas à elle seule une identity vérifiable.
3. Un callback est introduit avec sa target officielle, son rooting, son résultat, la durée de
   validité de son payload, son fault path et sa callback barrier.
4. Une operation asynchrone est introduite avec ses safety properties, ses liveness properties et
   les fairness assumptions qui restent externes.
5. Le modèle TLA+, les tests de conformance et le code évoluent dans la même PR. Le modèle commun
   décrit le contrat ; les modèles par binding en prouvent un refinement plus concret.
6. Les PR sans dépendance sémantique ne restent pas artificiellement dans le chemin critique de la
   stack.

## 2. Verdict sur les PR ouvertes

**Résumé.** Les PR #743, #744, #745 et #747 contiennent du travail réutilisable. La PR #746 porte en
revanche une frontière devenue contraire au design : elle expose les dépendances nécessaires pour
que la FFI pilote elle-même le connector, alors que cette machine doit d’abord vivre dans
`armonik-transport::runtime`. Elle doit être remplacée, pas corrigée par une couche supérieure.

**Intention.** Cette table doit permettre une décision de maintien ou de fermeture sans confondre
« beaucoup de modifications » et « intention de PR devenue fausse ».

| PR | Objet actuel | Décision proposée | Motif principal |
|---|---|---|---|
| [#743](https://github.com/aneoconsulting/ArmoniK.Api/pull/743) | le transport fournit un connector ; ArmoniK assemble son channel Tonic | **conserver** | la séparation connector/channel reste correcte et prépare un runtime HTTP indépendant de Tonic |
| [#744](https://github.com/aneoconsulting/ArmoniK.Api/pull/744) | skeleton ABI C, guards, handles, buffers, errors, runtime Tokio global | **conserver et amender avant merge** | les primitives sont utiles, mais identity, isolation runtime et shutdown doivent être fondés ici |
| [#745](https://github.com/aneoconsulting/ArmoniK.Api/pull/745) | génération C# des options | **conserver, hors du chemin critique** | la génération d’options ne décide ni scheduling ni callback protocol ; elle peut être reviewed séparément |
| [#746](https://github.com/aneoconsulting/ArmoniK.Api/pull/746) | reexports de Hyper, H2, Tokio et HTTP body vers le consumer du connector | **fermer et remplacer** | son intention place le request runtime dans le consumer FFI ; le bon niveau est `armonik-transport::runtime` |
| [#747](https://github.com/aneoconsulting/ArmoniK.Api/pull/747) | client/pool Hyper dans la FFI, configuration et rate limiter | **conserver mais restructurer** | builder, options et tests sont réutilisables ; l’exécution et le lifecycle doivent descendre dans le runtime transport |
| branche `wk/feat/rust-ffi-reactor` | request task, read/write, cancel, timeout, rate permit et protocole | **ne pas ouvrir telle quelle ; reprendre par morceaux** | la state machine est précieuse, mais `request.rs` mélange la machine fonctionnelle, l’ABI C et le callback contract |

Les PR CI #740 et #748 ne portent aucune abstraction du protocole et ne sont pas affectées par ce
plan.

La fermeture proposée de #746 n’est pas un jugement sur la qualité du code. C’est le seul cas où
l’intention complète de la PR devient fausse. La remplacer est plus lisible qu’un force-push qui
conserverait un numéro et un historique de review sans rapport avec le nouveau diff.

## 3. PR fondation à insérer après #743 : `armonik-transport::runtime`

**Résumé.** Une nouvelle PR doit être insérée entre #743 et #744. Elle reprend la partie
transport-specific du client et du reactor sans aucune notion de C, JNI, P/Invoke ou Python. Elle
offre une API Rust proche du contrat fonctionnel `Transport -> Connection -> Call`.

**Intention.** La machine HTTP/2 duplex doit être testable et utilisable en Rust sans connaître son
lowering FFI. C’est ce découpage, plus que la forme du callback, qui permet de réutiliser le code et
d’éviter qu’une seconde FFI recopie `request.rs`.

La PR introduit, sous un feature gate si nécessaire :

- un `Runtime`/executor adapter interne au transport ;
- un `Transport` configuré, puis une `Connection` logique ou un client/pool explicitement nommé ;
- plusieurs `Call` concurrentes sur la même connection logique ;
- request headers, request body demand, `complete(RequestTrailers)`, response headers, response body
  demand, response trailers et outcome terminal ;
- cancellation, timeout et rate permit avec leur ordering terminal ;
- une state machine sans callback C et sans raw pointer ;
- des tests de traces fonctionnelles, dont slow consumer, empty chunk, trailers, cancel/timeout et
  absence de driver progress sans demand.

Le mot `Connection` ne doit pas laisser entendre qu’un socket est ouvert lors de la configuration.
Si l’implémentation reste un pool Hyper lazy, le type et la documentation doivent dire qu’il s’agit
d’une connection logique pouvant créer et recycler plusieurs connexions physiques. Une même
connection logique accepte plusieurs calls concurrentes ; chaque call garde un counted reference
sur elle jusqu’à son terminal natif.

Les premiers modules TLA+ de cette PR sont indépendants de la FFI :

```text
Transport.tla       observations et commands fonctionnelles
CallLifecycle.tla   Configured, Started, HalfClosed, Terminal, Quiescent
FlowControl.tla     read/write demand, trailers et bounded in-flight data
```

Cette PR remplace la finalité de #746. Elle peut reprendre le code de #747 et de la branche reactor
par `git cherry-pick -n` ou déplacement de fichiers ; la réutilisation se mesure au comportement et
aux tests conservés, pas au maintien des fichiers dans leur crate actuel.

## 4. Remédiation de #744 : skeleton ABI et isolation runtime

**Résumé.** Les panic guards, error codes, blobs, buffers, génération de header et références
comptées de #744 sont à conserver. En revanche, le runtime Tokio process-wide ne doit pas être
confondu avec l’isolation runtime visible du protocole, et une raw address ne doit pas promettre la
détection fiable d’un stale handle après réutilisation.

**Intention.** #744 doit rendre impossibles, ou explicitement hors contrat, les classes d’erreurs
que toutes les PR suivantes seraient sinon obligées de traiter différemment.

Les modifications doivent être faites dans #744 avant son merge :

- introduire un `ak_runtime` explicite, owner des registries, start gate, callback configuration,
  fault latch, task group et callback barrier ;
- distinguer ce runtime d’isolation du moteur Tokio éventuellement partagé pour la durée du
  process ;
- définir `Running -> Stopping -> Draining -> Stopped` et
  `Failed -> FailedQuiescent | FailedUnquiesced` ;
- refuser la création d’un child dès que le start gate est fermé ;
- scope les handles par runtime et utiliser des slots generation-tagged pour toute identity qui
  traverse un callback ou doit être validée après un délai ;
- conserver les `Arc` internes pour empêcher la désallocation pendant un downcall, sans prétendre
  qu’ils résolvent à eux seuls le problème ABA ;
- définir la policy d’un handle inconnu, stale, wrong-owner ou wrong-generation : aucune
  dereference, fermeture du gate approprié, completion exactly-once des operations connues,
  diagnostics bornés et quiescence avant release ;
- documenter le contrat de chargement/déchargement de la DLL. Si le moteur Tokio vit jusqu’au
  process exit, l’unload dynamique doit être interdit ; sinon un join du moteur doit être fourni ;
- séparer payload borrowed-during-call, owned handle et input copié avant retour.

Le skeleton n’a pas encore besoin de connaître tous les events HTTP. Il doit néanmoins réserver les
concepts sur lesquels le protocole callback reposera : `runtime_ctx` rooted jusqu’à quiescence,
consumer token publié avant start, result code du callback, version et capability negotiation dans
les deux sens.

Les modules de preuve associés sont :

```text
FfiRuntime.tla          gates, children, callback in-flight et join
GenerationalIds.tla     allocation, validation, tombstones et non-confusion
FailureContainment.tla  détection, quarantine, quiescence et FailedUnquiesced
MemoryOwnership.tla     borrowed, copied, retained, released
```

La PR reste amendable : son intention — fournir les fondations de l’ABI — demeure exacte. Il faut
réécrire `runtime.rs` et une partie de `handle.rs`, mais pas fermer l’ensemble de la PR.

## 5. Remédiation de #745 : génération des options

**Résumé.** #745 peut rester presque inchangée, mais elle ne doit plus servir de base accidentelle
aux PR de protocole. Elle peut être rebased sur #744 amendée ou sortir de la chaîne critique.

**Intention.** Le générateur doit transformer un schema de données ; il ne doit pas devenir, par
proximité, le générateur implicite d’une state machine async qu’il ne sait pas exprimer.

La PR conserve la génération des types d’options, defaults et validation statique. Les règles de
lifecycle, fairness, callback threading, exceptions et ownership ne sont pas inférées du JSON
schema. À terme, un schema de protocole distinct pourra générer enums, layouts et skeletons de
routing, mais les proof obligations resteront des artefacts explicites.

## 6. Remédiation de #747 : client, configuration et parenté

**Résumé.** #747 doit devenir la projection C mince du client/connection fourni par la nouvelle PR
`armonik-transport::runtime`. La construction du connector, du pool, du timeout et du rate limiter
est déplacée ou encapsulée côté transport ; la FFI garde validation binaire, conversion et handles.

**Intention.** Le lifecycle complet de la connection doit être fixé avant la première request. Une
request ne doit pas ajouter après coup les références ou gates qui manquaient au client.

Les changements attendus sont :

- `ak_client_create` reçoit explicitement son `ak_runtime` parent ;
- le runtime refuse la création si son start gate est fermé ;
- le client garde un counted reference sur le runtime, et chaque request garde une référence sur le
  client et le runtime jusqu’à sa quiescence ;
- `ak_client_release` libère la référence du caller, sans invalider une request en vol ;
- le code précise création synchrone/configuration versus établissement lazy d’une connexion
  physique ;
- les limites de pool, concurrence, rate limiting et shutdown sont observables et testées ;
- les bytes du JSON sont borrowed uniquement pendant le downcall ; les strings/certificats requis
  après le retour sont owned par Rust ; les erreurs retournées ont un release symétrique.

Le builder, le schema, le rate limiter, le ledger de tests et une grande partie des tests de #747
sont conservables. Le fichier et les signatures changent substantiellement, mais la PR n’a pas à
être fermée : sa responsabilité « donner un client à l’ABI » reste correcte.

## 7. Reprise de la branche reactor

**Résumé.** La branche locale contient déjà les bons mécanismes fonctionnels : task duplex,
armement unitaire, read/write, cancel, timeout, rate permit et plusieurs tests de deadlock. Elle
doit être scindée par point de vue plutôt que merged comme un bloc FFI de 3 700 lignes.

**Intention.** L’équipe doit éviter deux extrêmes : jeter une state machine déjà éprouvée, ou
conserver son couplage actuel parce que les tests passent. La remédiation reprend les transitions et
les tests, puis rend explicites les contracts entre couches.

| Commit local | Élément à préserver | Nouvelle destination ou correction |
|---|---|---|
| `dc638ef0` | server fixture | tests de `armonik-transport::runtime`, réutilisé aussi par les tests FFI |
| `4ac35fb1` | request task, response/read | machine fonctionnelle dans le runtime transport ; adapter event dans la PR callback FFI |
| `bf6ba974` | write path | runtime transport avec `complete(RequestTrailers)` ; copie/ownership C dans l’adapter |
| `1ce1e37e` | cancel/release | cancellation fonctionnelle côté transport ; gates, roots et callback barrier côté FFI |
| `2bc8976f` | timeout | provider local d’un outcome terminal ; fairness du timer explicitée dans TLA+ |
| `287f7f7a` | rate permit | runtime transport, avec cancellation de l’attente et ownership du permit |
| `5bddfdf5` | correction du body bloqué | test de liveness conservé comme trace de conformance |
| `b873a7a4` | explication du protocole | réécriture normative après stabilisation des actions TLA+ |
| `430566dc` | guard write/close | invariant et test conservés dans la couche fonctionnelle puis projetés en ABI |

La reprise produit au moins deux PR :

1. la machine fonctionnelle Rust, intégrée à la PR runtime insérée après #743 ;
2. le lowering MsQuic-style dans `armonik-transport-ffi`, rebased sur #744 et #747 amendées.

Le lowering C introduit une signature conceptuellement équivalente à :

```c
ak_callback_result on_event(void *runtime_ctx,
                            ak_operation_token token,
                            const ak_event *borrowed_event);
```

Le `runtime_ctx` reste rooted jusqu’à la callback barrier. Le token est choisi et enregistré par le
binding avant `start`, de sorte qu’un callback avant le retour de `start` soit routable. Le payload
est valide pendant l’appel ; le trampoline le copie ou prend explicitement un owned reference avant
de retourner. Le callback ne complète pas de continuation utilisateur : il publie un
`CompletionRecord` dans une queue host, signale le scheduler et retourne.

La V1 n’autorise aucun downcall ordinaire reentrant depuis le callback. `read`, `write`, `complete`
et `cancel` sont postés après son retour. Un futur rearm inline serait une capability séparée,
ajoutée seulement avec preconditions et preuve de non-attente sur la même operation.

## 8. PR de binding et modèles TLA+ par langage

**Résumé.** Le trampoline est petit, mais il n’est pas identique en .NET, Java, C++ et Python. Le
contrat abstrait et la queue sont communs ; chaque binding fournit un refinement mince qui traite
son rooting, son scheduler et son shutdown réels.

**Intention.** « Même algorithme » ne doit pas devenir « mêmes assumptions cachées ». Chaque PR de
binding livre ensemble l’implémentation, le modèle de refinement et les tests de fault injection.

Le noyau commun modélise :

```text
CallbackProtocol.tla   callback borné, résultat et callback barrier
HostQueue.tla          publish-before-signal, bounded slots et exactly-once consumption
Composition.tla        décharge des assumptions entre Rust, FFI, binding et host
```

Puis chaque package fonctionnel ajoute :

| Binding | Implémentation minimale | Refinement propre |
|---|---|---|
| .NET Framework 4.8 | delegate rooted, `ConcurrentQueue<CompletionRecord>`, signal, worker/executor, `TaskCompletionSource` avec continuations non-inline | `DotNetBinding.tla` : delegate lifetime, dispose, scheduler rejection et TCS completion |
| Java 17 | JNI global refs, native or Java concurrent queue, dispatcher attaché, `CompletableFuture`, suppression de toute exception JNI pending | `JavaBinding.tla` : attach/detach, global-ref release, JVM shutdown et executor rejection |
| C++11 | MPSC queue, condition variable/executor, promise/future et RAII | `Cpp11Binding.tla` : object lifetime, thread join et broken promise |
| C++20 | même transport, projection `co_await`/async range sans changer l’ABI | `Cpp20Binding.tla` : coroutine frame lifetime et cancellation adapter |
| Python | queue native, `Py_AddPendingCall` ou signal d’event loop, acquisition du GIL hors callback Tokio, future Python | `PythonBinding.tla` : interpreter identity, finalization, GIL scheduling et dropped loop |

Les modèles par langage ne recopient pas `Transport.tla`. Ils instancient le modèle commun et ne
raffinent que les actions supplémentaires du binding. Les tests replayent les mêmes traces : fast
completion, unknown/stale token, duplicate terminal, queue saturation, exception/rejection du
scheduler, cancel race, shutdown et absence de callback tardif.

## 9. Ordre de rebase et de merge proposé

**Résumé.** La stack est reconstruite de bas en haut. Le seul travail fermé est #746 ; les autres
PR sont rebased après amendement de leur fondation.

**Intention.** Cet ordre garantit qu’une review peut vérifier un invariant à l’endroit où il naît,
et que la branche reactor ne devient jamais une dépendance opaque de la preuve.

1. Merger #743 après sa review actuelle.
2. Fermer #746 et ouvrir sa remplaçante sur #743 : `armonik-transport::runtime` avec state machine
   fonctionnelle et premiers modules TLA+.
3. Rebase et amender #744 sur cette fondation : runtime d’isolation, IDs generation-tagged,
   ownership, shutdown et failure containment.
4. Sortir #745 du chemin critique ; la rebase sur #744 n’est requise que si ses artefacts en
   dépendent effectivement.
5. Rebase et restructurer #747 : projection C du client/connection et parenté avec `ak_runtime`.
6. Ouvrir le lowering callback FFI à partir des commits utiles de la branche reactor, avec
   `CallbackProtocol.tla`, tests de conformance et fault injection.
7. Stabiliser `TRANSPORT.md`, `PROTOCOL.md` et `DESIGN.md` dans cette PR, lorsque les actions du
   modèle et les signatures se correspondent.
8. Ouvrir le binding .NET de référence, puis Java 17. C++11/C++20 et Python suivent avec leurs
   refinements propres.

Les rebases seront coûteuses une fois, mais elles évitent de maintenir une ABI temporaire,
un handle registry temporaire et une seconde voie waitable que la décision de V1 n’utilise pas.

## 10. Conditions de fermeture supplémentaires

**Résumé.** Aucune autre PR n’a aujourd’hui besoin d’être fermée. Certaines parties doivent être
réécrites, mais leur intention reste compatible avec la cible.

**Intention.** Une fermeture doit signaler une intention architecturale abandonnée, pas simplement
un gros diff de remédiation.

- Fermer #744 seulement si l’équipe veut conserver publiquement son runtime implicite et ses
  pointer identities tels quels. Dans ce cas, une nouvelle ABI incompatible est plus honnête ; ce
  document recommande au contraire d’amender la draft.
- Fermer #747 seulement si l’équipe refuse de déplacer le client fonctionnel dans
  `armonik-transport::runtime`. Sinon sa projection, son schema et ses tests justifient sa reprise.
- Ne pas fermer la branche reactor : elle n’a pas de PR. Ne pas l’ouvrir telle quelle ; extraire ses
  commits et conserver leur attribution lors du split.
- Si l’équipe revient sur la cible `armonik-transport::runtime` et décide que chaque FFI pilotera
  directement le connector, alors #746 peut être conservée. Ce choix augmente volontairement la
  duplication entre FFI et contredit la factorisation fonctionnelle décrite dans `DESIGN.md`.

## 11. Exit criteria de la remédiation

**Résumé.** La remédiation est terminée lorsque le système composé garantit ses invariants et que
ses seules assumptions résiduelles sont générales : CPU/scheduler progress, peer-or-timeout et
terminaison du user code que l’API décide d’attendre.

**Intention.** « Les tests passent » ne suffit pas ; chaque couche doit dire ce qu’elle garantit à
l’extérieur et quelles guarantees des couches voisines déchargent ses assumptions.

Avant stabilisation ABI, il faut au minimum :

- un mapping bijectif `action TLA+ -> transition de code -> test ou runtime check` ;
- zéro unknown/stale/wrong-owner token dereferenced ;
- publication du host state avant tout callback possible ;
- aucun user code et aucune continuation inline depuis un worker Tokio ;
- toute mémoire libérable uniquement par son allocateur, après fin de ses borrows et de ses
  callbacks in-flight ;
- un outcome terminal host exactly-once pour chaque call acceptée, sous le fairness profile déclaré ;
- fermeture des start gates avant quarantine ou shutdown ;
- quiescence vérifiable, ou résultat explicite `FailedUnquiesced` sans free dangereux ;
- aucun callback après la callback barrier ;
- tests .NET et Java couvrant rooting, exception containment, queue saturation, scheduler rejection,
  GC/JVM pause, cancel race et shutdown ;
- benchmarks du chemin retenu complet. Une variante waitable n’est ajoutée que si un résultat ou
  une cible invalide concrètement le trampoline.
