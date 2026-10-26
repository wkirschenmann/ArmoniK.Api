# État des branches — moteur gRPC natif (Rust) et sa liaison .NET

Point au 26 octobre 2026. Ce rapport décrit où en est le travail sur le canal gRPC natif d'ArmoniK.Api :
le moteur Rust (`armonik-transport`, `armonik-transport-ffi`), sa liaison .NET
(`ArmoniK.Api.Client.RustGrpcChannel`) et le modèle TLA+ qui les spécifie
(`spec/armonik_grpc_ffi/tla`, niveaux 0, 1 et 2).

La branche `status` part de la tête de la branche de travail (`c3809c500`). Elle ajoute ce seul fichier.

## 1. Où se trouve quoi

| Branche | Où | Tête | Rôle |
|---|---|---|---|
| `wk/feat/phase1-audit` | fork `wkirschenmann/ArmoniK.Api` | `c3809c500` (2026-10-23) | **La branche de travail à jour.** Tout ce qui est intégré est ici. |
| `wk/feat/phase1` | fork | `9025febf6` | Ancienne tête, ne plus s'y fier. |
| `wk/feat/phase1-wip`, `wk/feat/phase1-full-history` | fork | — | Historiques figés, ne pas y pousser. |
| `archive/2026-10-09/*` | fork | — | Les 133 branches d'exploration archivées le 2026-10-09. |
| `main` | `aneoconsulting/ArmoniK.Api` | `83b1aa52f` (2026-10-23) | Rien du moteur n'y est fusionné. |

Écart avec `main` : la branche de travail a **524 commits** que `main` n'a pas. `main` en a **40** que la branche de travail
n'a pas : la CI C++ sous Windows, la publication des crates sur crates.io, protox à la place de protoc, async-trait 0.1.92, et les
bumps de version 3.29.3 et 3.29.4. Il faudra fusionner `main` avant toute PR.

Rien n'est encore proposé en PR. Le plan est le suivant :

1. Migrer les branches d'exploration vers le dépôt privé `ArmoniK.Community.Private`.
2. Extraire le travail sur une branche propre d'ArmoniK.Api (T6.9).
3. Écrire le README et le guide de migration.
4. Brancher le packaging dans la CI (T6.12).

## 2. Ce que contient la branche de travail

**Fonctionnel.** Les tâches T0 à T6, T7.1, T10.1 et T10.2 sont faites, selon `spec/armonik_grpc_ffi/tasks.md`.

- **Modèle TLA+, trois niveaux, prouvés avec TLAPS.**
  - niveau 0 : le contrat gRPC ;
  - niveau 1 : l'ABI C ;
  - niveau 2 : la liaison .NET.

  Le niveau 2 raffine le niveau 1, qui raffine le niveau 0. Le code raffine le modèle, et le modèle prime sur `design.md`.
- **Toutes les formes d'appel** : unaire, flux client, flux serveur, bidirectionnel.
- **Échéances, retry et replay** (gRFC A6), avec étranglement adaptatif.
- **Attente de disponibilité** (wait-for-ready) par appel, et connexion anticipée en option.
- **Plafond mémoire**, qui compte aussi les messages reçus.
- **Pool de connexions HTTP/2** sous un même canal, réglé par `Http2.SimultaneousCallsPerConnection`.
- **TLS et proxy** :
  - TLS (CA, chaîne complète, identité PKCS#12, magasin Windows) ;
  - proxy explicite, d'environnement ou système Windows.
- **Options.** Un schéma JSON unique (`options.schema.json`, `runtime.schema.json`) dont le type C# est généré. Le chargeur refuse toute clé
  inconnue, et un seul chargeur sert tous les hôtes.
- **ABI générée depuis le Rust**, par cbindgen pour l'en-tête et csbindgen pour le P/Invoke.
- **Intégration dans `ArmoniK.Api.Client`.** `GrpcClient.Transport` vaut `Managed` (grpc-dotnet, par défaut) ou `Native`.
- **Observabilité.** Journaux, configuration effective, et métriques derrière la feature cargo `metrics` : les deux builds natifs sont
  dans le paquet NuGet.
- **Cibles livrées.**
  - Systèmes : Windows et Linux, en x86, x64 et arm64.
  - Frameworks : .NET 8 et suivants, .NET Framework 4.7.2 et 4.8.
  - Sous win-arm64, le moteur est compilé avec LSE, et la liaison refuse un processeur sans les atomiques Armv8.1.

**Performance, intégré du 17 au 23 octobre.**

| Changement | Ce qui a été mesuré | Commit |
|---|---|---|
| LTO « fat » par défaut | Téléversement unaire +21 %, CPU par Mio −18 %, DLL −7 % ; construction 35 à 45 % plus longue | `886101e3b` |
| Niveaux de jeu d'instructions (v2, v3, native) | Aucun gain reproductible : 19 comparaisons sur 312 vont dans le même sens, le hasard en donne 19,5 | `886101e3b` |
| Moteur x86-64-v4 (AVX-512) à côté du moteur de base, sur win-x64 et linux-x64 | Choisi au chargement par une petite sonde native, `armonik-cpu-probe`, sous .NET 8+ comme sous .NET Framework. **Jamais exécuté** : pas de machine AVX-512 disponible. | `b17e90b2f`, `e781b0961` |
| gzip via zlib-rs au lieu de miniz_oxide | Téléversement +19 à +44 %, CPU client par Mio −26 à −28 % ; compression un peu meilleure (2,537 contre 2,445) | `c3809c500` |
| zstd compilé par clang-cl | Mesuré puis **écarté** : aucun gain reproductible sur x64, +22 % seulement en x86 | `c3809c500` |
| Bancs | Client et serveur épinglés sur des cœurs séparés ; scénario mixte mTLS (gros téléversements à côté d'appels unaires) | `03896ac6d`, `3ef224344`, `0dd1139b4` |

Les mesures sont dans `spec/armonik_grpc_ffi/benchmarks.md`, et les décisions datées dans `decisions.md`.

## 3. Ce qu'on a appris sur les performances

La charge qui compte est mTLS, avec des téléversements en flux client de plus de 100 Mo à côté de nombreux petits appels unaires
concurrents, sur un seul canal logique.

- **Un canal natif fait tout sur un seul thread.** Ce thread porte la connexion HTTP/2, les appels et les rappels vers .NET. Il dépense
  environ 45 µs de CPU par appel, ce qui plafonne un canal vers 22 000 appels/s. À 64 appels en vol en h2c, grpc-dotnet fait 56 000
  appels/s, mais en dépensant environ 114 µs par appel sur environ 6 cœurs. C'est le premier frein, et c'est l'objet du pool de threads
  (§4).
- **Sur la charge mixte, le natif ne gagne que sur les téléversements seuls.**

  | Mesure | Natif | grpc-dotnet |
  |---|---|---|
  | Téléversement seul | 358 Mio/s | 105 Mio/s |
  | CPU client par Mio, téléversement seul | 2,95 ms | 11,4 ms |
  | 1 téléversement à côté de 16 unaires | 8 253 appels/s | 17 952 appels/s |

  Sous .NET Framework 4.8, seul le natif est utilisable : grpc-dotnet y fait 957 appels/s.
- **Le banc sur la boucle locale fausse le cas de plusieurs flux sur une même connexion.** Sur la boucle locale, le MSS vaut 65 495 octets
  face à une fenêtre de 256 Kio. Dans ces conditions, la protection anti « silly window » de TCP sous Windows retient des envois
  pendant environ 20 ms, ce qui fait chuter h2-batch avec plusieurs téléversements par connexion.

  Sur un vrai chemin réseau (serveur dans WSL2, MSS de 1 460), 16 trames par écriture est au contraire le réglage le plus rapide et le
  moins coûteux. Il reste donc le défaut. **Les cas à plusieurs flux doivent se mesurer hors boucle locale.**
- **Jeux d'instructions et dépendances C.** Seuls ring (TLS) et zstd-sys contiennent du C ou de l'assembleur. Les chemins chauds de
  ring et le memcpy du C choisissent déjà leur variante à l'exécution. ring reste le plus rapide sur ce processeur : 6 332 Mio/s en
  AES-256-GCM, contre 3 851 pour aws-lc-rs et 3 259 pour graviola. Hors compression, il n'y a presque rien à gagner de ce côté.
- **La part du noyau dans le CPU client est de 28 à 58 %**, selon la comptabilité de l'OS. Le chiffre de 81 à 86 % d'un rapport
  antérieur venait d'un échantillonneur biaisé.

## 4. Travail en cours, hors de la branche de travail

Ces branches sont **locales**, dans des worktrees de la machine de développement. Elles ne sont pas sur le fork.

| Branche locale | Commits | État |
|---|---|---|
| `wk/perf/runtime-thread-pool` | 3, de `1d7c05463` à `ac8d8087a` | Pool de threads au niveau du runtime, implémenté et testé. Campagne de mesure pas encore lancée. |
| `wk/perf/c1-hold-per-wave` | 1, `a182776b8`, un test | Regroupement des écritures par vague (C1). Code moteur non commité. En pause. |

**Pool de threads (`wk/perf/runtime-thread-pool`).** C'est la décision du 2026-10-18 : on revient sur « un thread par canal ».

- **Le principe.** Le runtime possède un pool de threads de travail. Chaque connexion physique est attachée à un thread à sa création,
  et un thread sert des connexions de plusieurs canaux.
- **Les options.**
  - `WorkerThreads` vaut `"Auto"` ou `{"Count": n}`. `"Auto"` compte le masque d'affinité du processus, parce que Rust l'ignore sous
    Windows.
  - `WorkerAssignment` vaut `"LeastConnections"` (par défaut) ou `"RoundRobin"`.
- **Le placement.** Un appel réserve sa place avant que sa tâche soit lancée, et il ne change jamais de thread. Il fait ses nouvelles
  tentatives sur son propre thread.
- **Le modèle TLA+.** Vérifié niveau par niveau, il ne change pas.
- **Un coût accepté.** Un rappel hôte qui ne rend pas la main gèle tout son thread, et donc toutes les connexions qu'il porte, de tous
  les canaux.
- **Les tests.**
  - Tests release : 744 passent sur h2 d'origine, 745 sur h2-batch.
  - Tests de la liaison : 355 sur net8.0, 351 sur net4.8 et 351 sur net4.7.
- **Ce qui reste dans le worktree, non commité.**
  - Les options `pool=` et `workers=` du banc.
  - La documentation : abi.md, architecture.md, call-shapes.md, decisions.md, observability.md, requirements.md.
- **Ce qui reste à faire.**
  1. La campagne : 1, 2, 4 ou 8 threads, croisés avec `FromServer` et `Limit` 1, 4 et 16, sur les scénarios concurrent, mixte et appel
     isolé, en 5 passes.
  2. Puis la conception d'une limite d'appels par connexion qui s'ajuste aux performances mesurées.

**C1, écritures regroupées par vague (`wk/perf/c1-hold-per-wave`).** Le chemin d'écriture saute les réveils de la connexion jusqu'à la
fin du tour et écrit une fois.

| Mesure | Avec C1 | Sans C1 |
|---|---|---|
| 64 appels en vol, h2c | 26 880 appels/s | 16 340 appels/s |
| 64 appels en vol, TLS | 25 710 appels/s | 18 560 appels/s |
| Appel isolé, P50 | 599 µs | 711 µs |
| CPU serveur par appel, à 64 en vol | ×1,9 | référence |
| 1 téléversement à côté de 64 unaires, sur la boucle locale | 26 Mio/s | 53 Mio/s |

Le coût côté serveur vient d'écritures 2,6 fois plus fréquentes. Une variante qui laisserait passer les données de gros flux est à
essayer, mais seulement **après** la fusion du pool de threads, et en mesurant aussi hors boucle locale.

## 5. Ce qui reste ouvert

- **T6.16, à moitié fait.** Le redimensionnement des tampons de l'hôte est fait ; la fenêtre d'envoi supérieure à 1 ne l'est pas. La
  liaison .NET n'autorise qu'une écriture en vol par appel, si bien que chaque morceau d'un téléversement fait un aller-retour par le
  thread du canal.
- **T6.9, documentation et nettoyage**, sur une branche propre, puis **T6.12**, le packaging dans la CI. La CI devra construire le
  moteur h2-batch, les builds v4 et la sonde CPU, et ajouter une jambe arm64 avec LSE.
- **Portage en amont.** T9.1 couvre les correctifs de tunnel que hyper-util n'a pas encore eus, et T9.2 les écarts de tonic par rapport
  aux documents gRPC.
- **Pour la V2.** T10.3, les traces.
- **Mesure manquante.** Aucune mesure de v4 sur une machine AVX-512.
- **Test instable connu.** `a_call_cancelled_after_its_end_keeps_its_half_close` échoue parfois sur h2 d'origine, et passe quand on le
  relance seul.

## 6. Pour démarrer

- Les documents qui gouvernent le projet sont dans `spec/armonik_grpc_ffi/` :
  - `requirements.md` et `design.md` ;
  - `abi.md` et `call-shapes.md` ;
  - `formal-model.md` et `tla/` ;
  - `decisions.md`, où chaque choix est daté avec sa raison ;
  - `tasks.md` et `benchmarks.md`.
- La règle de préséance : **TLA+ prime sur `design.md`, qui prime sur `tasks.md`**. Le code raffine le modèle, et une correction n'est
  jamais un contournement documenté.
- Les tests de la liaison tournent sur net8.0, net4.8 et net4.7. Le moteur Rust se teste en release, sur h2 d'origine et sur h2-batch.
