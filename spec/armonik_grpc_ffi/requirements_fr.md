# Requirements — Client .NET ArmoniK sur channel gRPC natif Rust

## Introduction

ArmoniK est une plateforme de calcul distribué. Son client .NET (`ArmoniK.Api.Client`) communique
avec le serveur ArmoniK via gRPC pour soumettre des sessions, des tasks et récupérer des résultats.

Aujourd'hui, le transport gRPC repose sur des composants managés .NET (`Grpc.Net.Client`,
`WinHttpHandler`, `SocketsHttpHandler`, `GrpcWebHandler`). Ce choix génère trois problèmes :

1. **Comportement divergent selon le runtime .NET.** .NET Framework 4.7.2/4.8 utilise WinHTTP
   (avec un fallback gRPC-Web), tandis que .NET 6+ utilise SocketsHttpHandler. Les timings, les
   erreurs et les capacités diffèrent silencieusement.
2. **Options inopérantes.** `RequestTimeout` et `OverrideTargetName` sont déclarées mais n'ont
   aucun effet (warning seulement).
3. **Limitations de plateforme.** WinHTTP ne parle pas HTTP/2 de manière fiable sur toutes les
   versions de Windows ciblées, d'où le fallback gRPC-Web.

Le projet consiste à remplacer ce transport par un **channel gRPC natif écrit en Rust**, commun à
toutes les plateformes, exposé à .NET via une FFI C mince. L'architecture est en 5 couches :

```
armonik-transport (Rust, réseau)
  → armonik-grpc-channel (Rust, sémantique gRPC)
    → armonik-grpc-channel-ffi (Rust → ABI C, isolation runtime)
      → ArmoniK.Api.Client.RustGrpcChannel (C#, CallInvoker natif)
        → ArmoniK.Api.Client (C#, stubs gRPC, API publique inchangée)
```

Le développeur applicatif ne voit aucun changement d'API. Le comportement réseau devient identique
quelle que soit la version du runtime .NET. Les options deviennent toutes effectives.

Ce document formalise les requirements de la V1 de ce projet, structurés par persona.

## Glossaire

| Terme | Définition |
|-------|-----------|
| Channel | Ressource logique représentant une connexion gRPC vers un endpoint. Peut multiplexer plusieurs calls HTTP/2. |
| Call | Un appel gRPC en cours (requête + réponse), associé à une cardinalité (unary, client streaming, server streaming, bidi). |
| CallInvoker | Abstraction .NET (`Grpc.Core.CallInvoker`) qui permet aux stubs gRPC générés d'émettre des calls sans connaître le transport. |
| Metadata | Paires clé/valeur transmises dans les headers HTTP/2 (request metadata, initial metadata, trailing metadata). |
| Trailing metadata | Headers HTTP/2 envoyés en fin de stream (trailers). Côté réponse, ils contiennent `grpc-status`, `grpc-message` et metadata utilisateur. Côté requête, HTTP/2 autorise des request trailers (envoyés avec `END_STREAM` après le body), mais gRPC ne les utilise pas en pratique — notre API ne les expose pas en V1. |
| end_send | Signal que le client n'enverra plus de messages sur cette call. Correspond à `END_STREAM` HTTP/2 sur le request body. |
| Deadline | Durée maximale d'une call, appliquée localement et transmise au serveur via le header `grpc-timeout`. |
| Runtime (FFI) | Instance Tokio possédée par la couche FFI, qui exécute les tasks réseau et gère le lifecycle (shutdown, quiescence). |
| Quiescence | État dans lequel aucune task, callback ou thread natif n'est en vol. Condition nécessaire et suffisante pour l'unload. |
| Trampoline | Fonction callback C minimale qui copie un payload et le publie dans une queue host en exécutant le moins de code possible. |
| Host | Le processus appelant (ici le runtime .NET) qui charge la bibliothèque native et consomme ses events. |
| Consumer token | Identifiant choisi par le binding, publié dans sa registry avant `call_start`, utilisé pour router les events. |
| Demand-driven | Le client contrôle le rythme de réception des messages en demandant explicitement les suivants (`request_messages`). |

---

## Personas

### Développeur applicatif ArmoniK

Utilise le client .NET pour soumettre des sessions, tasks et récupérer des résultats. Responsable
du déploiement en production et de fournir les éléments d'observabilité à ses équipes ops.

### Membre de l'équipe ArmoniK

Maintient le client `ArmoniK.Api.Client`, développe le binding .NET (`RustGrpcChannel`), développe
les crates Rust (transport, channel, FFI). Responsable de la qualité, de la compatibilité et de
l'évolution du stack.

---

## Requirement 1 : Appels gRPC fonctionnels

**User Story:** En tant que développeur applicatif, je veux que mes appels aux services ArmoniK
(sessions, tasks, results, events, health) fonctionnent correctement via le channel natif, afin de
ne pas avoir à modifier mon code applicatif.

### Acceptance Criteria

1. Les 4 cardinalités gRPC (unary, client streaming, server streaming, bidirectional streaming)
   sont supportées, en mode async et en mode bloquant (unary bloquant via `BlockingUnaryCall`).
2. Les stubs C# générés par protobuf fonctionnent sans modification avec le nouveau `CallInvoker`.
3. Les request metadata sont transmises au serveur dans les headers HTTP/2 de la requête.
4. Les initial metadata de la réponse (headers HTTP/2) sont accessibles au client.
5. Les trailing metadata (trailers HTTP/2, incluant `grpc-status` et `grpc-message`) sont
   accessibles au client.
6. Les messages gRPC sont correctement framés (length-prefixed) et déframés.
7. Le client peut envoyer un ou plusieurs messages et signaler la fin d'envoi (`end_send`).
8. Le client peut recevoir un ou plusieurs messages de la réponse.
9. Un unary call retourne le résultat ou une erreur gRPC (status non-OK).
10. Un status `trailers-only` (réponse sans body, status dans les headers) est géré correctement.

---

## Requirement 2 : Cancellation et deadline

**User Story:** En tant que développeur applicatif, je veux pouvoir annuler un appel en cours ou
imposer un timeout, afin de ne pas bloquer mon application sur un serveur qui ne répond pas.

### Acceptance Criteria

1. Un `CancellationToken` annulé provoque l'annulation de la call (RST_STREAM envoyé au serveur).
2. Le call annulée se termine avec un status `CANCELLED`.
3. Une deadline (configurée au niveau channel ou par call) est appliquée localement.
4. La deadline est transmise au serveur via le header `grpc-timeout`.
5. Si la deadline expire avant la réponse, la call se termine avec un status `DEADLINE_EXCEEDED`.
6. L'option `RequestTimeout` du client existant produit une deadline effective (plus seulement un
   warning).

---

## Requirement 3 : Retry automatique

**User Story:** En tant que développeur applicatif, je veux que les appels échoués sur des erreurs
transitoires soient retentés automatiquement, afin d'améliorer la résilience sans code applicatif
supplémentaire.

### Acceptance Criteria

1. Les calls qui échouent avec les status `UNAVAILABLE`, `ABORTED` ou `UNKNOWN` sont retentées.
2. La configuration par défaut est : 5 attempts au total, backoff initial 1s, maximum 5s,
   multiplicateur 1.5.
3. La configuration de retry est paramétrable.
4. Une call en streaming client n'est retentée que si le volume de données envoyées tient dans
   un buffer de rejeu configurable. Au-delà, la call est considérée committed.
5. Une call bidirectionnelle est retentable tant qu'aucune réponse (initial metadata ou message)
   n'a été reçue ET que le buffer de rejeu n'est pas dépassé.
6. Un unary call est retentable tant que la réponse n'a pas commencé.
7. Le retry respecte la deadline globale de la call (pas de retry si la deadline restante est
   insuffisante pour le backoff).
8. Le nombre total d'attempts inclut la tentative initiale (5 attempts = 1 initial + 4 retries).
9. La taille du buffer de rejeu est configurable (par channel). Un buffer à 0 rend les calls
   streaming non retentables dès le premier message envoyé.

---

## Requirement 4 : TLS et authentification

**User Story:** En tant que développeur applicatif, je veux pouvoir me connecter à un serveur
ArmoniK sécurisé par TLS avec différentes configurations de certificats, afin de respecter la
politique de sécurité de mon infrastructure.

### Acceptance Criteria

1. La connexion TLS utilise les racines de confiance du système par défaut.
2. Un certificat CA PEM explicite peut être fourni par chemin de fichier pour valider le serveur.
3. Le mode mTLS est supporté avec un certificat client fourni par chemin :
   - P12/PFX (avec mot de passe optionnel)
   - PEM + clé séparée
4. Le mode mTLS supporte aussi la résolution du certificat client depuis le Windows Certificate
   Store (X509Store), identifié par Thumbprint, SubjectName ou FriendlyName. La résolution est 
   effectuée côté Rust.
5. Le mode connexion non vérifiée (insecure) est disponible en opt-in explicite.
6. L'option `OverrideTargetName` modifie effectivement le nom vérifié par le handshake TLS.
7. Une erreur TLS produit un message d'erreur diagnosticable (sans exposer de secrets : paths de
   clé privée, mots de passe).

---

## Requirement 5 : Proxy

**User Story:** En tant que développeur applicatif, je veux que le client respecte la configuration
de proxy de mon environnement, afin que mes appels gRPC puissent traverser les infrastructures
réseau d'entreprise.

### Acceptance Criteria

1. Le proxy peut être désactivé explicitement.
2. Un proxy explicite peut être configuré par URL, avec username et password en champs séparés.
   Si des credentials sont fournis séparément, l'URI du proxy ne doit pas contenir de
   userinfo (user:password dans l'URL) — c'est une erreur de configuration.
3. Le proxy peut être lu depuis les variables d'environnement (`HTTP_PROXY`, `HTTPS_PROXY`,
   `NO_PROXY`).
4. Sous Windows, le proxy système (configuré dans les paramètres réseau) est lisible.
5. L'authentification proxy Basic est supportée.
6. La résolution du proxy système Windows ne bloque pas le thread appelant (async avec timeout).

---

## Requirement 6 : Connexion et pool

**User Story:** En tant que développeur applicatif, je veux que la gestion des connexions soit
transparente et efficace, afin de ne pas avoir à gérer manuellement les connexions HTTP/2.

### Acceptance Criteria

1. La création du channel est synchrone et ne provoque pas d'I/O réseau par défaut (connexion
   lazy).
2. Une option de configuration permet une connexion eager (HTTP/2 établie dès la création du
   channel). Le choix lazy/eager est une option de configuration.
3. Plusieurs calls concurrentes sont multiplexées sur la même connexion HTTP/2.
4. Le pool de connexions gère le keepalive TCP et le timeout d'inactivité (idle timeout).
5. Un connect timeout est configurable et appliqué à l'établissement de la connexion.
6. Un même process peut créer plusieurs channels vers des endpoints différents.
7. Les channels partagent un seul runtime natif (un seul thread pool Tokio par process).

---

## Requirement 7 : Comportement unifié cross-runtime

**User Story:** En tant que développeur applicatif, je veux que mon application ait le même
comportement réseau que je la déploie sur .NET Framework 4.7.2, 4.8, .NET 6 ou .NET 8, afin
d'éliminer les bugs spécifiques à une plateforme.

### Acceptance Criteria

1. Le même binaire applicatif (netstandard2.0) produit le même comportement réseau sur .NET
   Framework 4.7.2, 4.8, .NET 6 et .NET 8.
2. Les options de configuration produisent le même effet quelle que soit la plateforme .NET.
3. Le transport natif ne dépend pas de `WinHttpHandler`, `SocketsHttpHandler` ou
   `GrpcWebHandler`.
4. Les limitations connues de WinHTTP (pas de HTTP/2 direct sur certaines versions) ne s'appliquent
   plus.
5. Aucun fallback conditionnel n'est nécessaire dans le code applicatif.

---

## Requirement 8 : Packaging et déploiement

**User Story:** En tant que développeur applicatif, je veux que le channel natif soit distribué via
NuGet et fonctionne sans configuration manuelle, afin de l'adopter avec un simple changement de
dépendance.

### Acceptance Criteria

1. Le package NuGet contient les binaires natifs pour win-x64, win-x86, linux-x64, linux-x86 et
   linux-arm64.
2. Le binaire natif approprié est résolu automatiquement au runtime selon le RID.
3. Aucune installation supplémentaire (runtime Rust, DLL externe) n'est nécessaire.
4. Si la DLL native est absente (RID non supporté), la création du channel échoue avec une
   erreur explicite (pas de fallback silencieux).

---

## Requirement 9 : Compatibilité API

**User Story:** En tant que développeur applicatif, je veux que le passage au channel natif ne
casse pas mon code existant, afin de migrer sans effort de développement.

### Acceptance Criteria

1. L'API publique de `ArmoniK.Api.Client` ne change pas (mêmes classes, mêmes méthodes).
2. Les options de configuration existantes (`GrpcChannel`, endpoint, TLS, proxy, timeout) sont
   préservées et mappées vers le channel natif.
3. Le `CallInvoker` natif est injectable là où un `CallInvoker` est attendu.
4. Les patterns d'utilisation existants (DI, factory, configuration par options) sont conservés.

---

## Requirement 10 : Shutdown et dispose

**User Story:** En tant que développeur applicatif, je veux que la fermeture de mon application
soit propre et déterministe, afin d'éviter les fuites de ressources ou les crashes à l'arrêt.

### Acceptance Criteria

1. Le channel/runtime implémente `IAsyncDisposable`.
2. `DisposeAsync` déclenche un shutdown ordonné : arrêt des nouvelles calls, drain des calls
   en cours (ou annulation), attente de quiescence.
3. Après dispose, aucun thread natif ni callback n'est en vol.
4. Un dispose pendant des calls actives les annule proprement (status CANCELLED).
5. L'unload de la DLL native est sûr après dispose (certifié par l'état `AK_RELEASED`).
6. Un timeout de dispose ne force pas l'unload — il laisse le runtime en état non-quiescent
   plutôt que de risquer une corruption.

---

## Requirement 11 : Diagnostics et erreurs

**User Story:** En tant que développeur applicatif responsable de la mise en production, je veux
des messages d'erreur exploitables quand la connexion échoue, afin de diagnostiquer les problèmes
sans lire le code source du transport.

### Acceptance Criteria

1. Une erreur de configuration (endpoint invalide, certificat introuvable, option inconnue) est
   rapportée immédiatement à la création du channel avec un message explicite.
2. Une erreur de connexion (DNS, TCP, TLS handshake) est rapportée avec la cause complète
   (cause chain aplatie en un message UTF-8).
3. Les messages d'erreur ne contiennent pas de secrets (paths de clé privée, mots de passe,
   contenu de certificats).
4. Les messages d'erreur ne contiennent pas d'informations de debug internes (source locations,
   adresses mémoire) en build release.
5. Les codes d'erreur sont discriminables programmatiquement (configuration vs connexion vs
   transport vs timeout vs annulation).

---

## Requirement 12 : Contrat public du connecteur (équipe ArmoniK)

**User Story:** En tant que membre de l'équipe ArmoniK, je veux que la frontière publique entre
`armonik-transport` et `armonik-grpc-channel` soit formalisée et stable, afin de pouvoir
développer les deux crates indépendamment.

### Acceptance Criteria

1. Le type/trait public du connecteur est documenté avec ses garantees et son contrat d'erreur.
2. Les options de configuration sont réparties selon leur propriétaire sémantique : transport
   (endpoint, TLS, TCP, proxy, connect timeout) vs channel (retry, deadline, HTTP/2, pool).
3. Le connecteur ne dépend d'aucune notion gRPC (pas de status, pas de retry, pas de deadline).
4. Le channel ne réinterprète pas les options transport (pas de double résolution endpoint/TLS).
5. Le contrat est testé : un changement incompatible dans le connecteur casse un test en amont.
6. Le client Rust ArmoniK (`armonik::Client`) utilise `armonik-grpc-channel` comme transport
   (via un adapter Tonic ou directement), partageant le même moteur gRPC que la FFI expose aux
   bindings.

---

## Requirement 13 : ABI stable et versionnée (équipe ArmoniK)

**User Story:** En tant que membre de l'équipe ArmoniK, je veux que l'ABI C soit versionnée et
additive, afin qu'un binding compilé contre une version antérieure continue de fonctionner avec
une bibliothèque plus récente.

### Acceptance Criteria

1. `ak_abi_version()` retourne la révision de l'ABI.
2. Un binding vérifie la version au chargement et diagnostique un mismatch.
3. Les ajouts (nouveaux entry points, event kinds, status codes) sont additifs : un binding
   ancien ignore ce qu'il ne connaît pas.
4. Aucun changement de signature, layout, convention d'appel ou sémantique d'un symbole existant
   dans une même version majeure.
5. Les records évolutifs utilisent `size` + `version` + `flags` + champs réservés validés à zéro.

---

## Requirement 14 : Safety et lifecycle FFI (équipe ArmoniK)

**User Story:** En tant que membre de l'équipe ArmoniK, je veux que la couche FFI garantisse
l'absence de corruption mémoire et un lifecycle déterministe des ressources, afin d'éviter les
crashes, les fuites et les comportements indéfinis lors de l'utilisation depuis .NET.

### Acceptance Criteria

1. Un handle invalide (déjà libéré, inconnu, mauvais type) est rejeté avec une erreur — jamais
   un accès mémoire invalide.
2. Les données passées du host vers Rust ne sont jamais lues après le retour du downcall.
3. Les notifications (events) pour une call sont délivrées une à la fois (sérialisées). Les
   notifications de calls différentes peuvent être concurrentes.
4. Chaque call acceptée produit exactement un event terminal.
5. Aucun event n'est délivré après l'event terminal d'une call.
6. Après le shutdown du runtime, aucune création de ressource (channel, call) n'est acceptée.
7. L'unload de la bibliothèque native n'est autorisé qu'après quiescence certifiée (`AK_RELEASED`).
8. Un panic côté Rust ne se propage pas au host — il est contenu et converti en erreur.

---

## Requirement 15 : Modèle formel TLA+ (équipe ArmoniK)

**User Story:** En tant que membre de l'équipe ArmoniK, je veux un modèle TLA+ qui vérifie les
propriétés de safety et de liveness du protocole FFI, afin d'avoir confiance dans la correction
du design avant l'implémentation.

### Acceptance Criteria

1. Les modèles TLA+ vivent dans `spec/armonik_grpc_ffi/tla/`.
2. La spec abstraite (`AbstractGrpc.tla`) décrit le contrat fonctionnel observable (calls,
   terminal, cancel, retry).
3. La spec FFI (`FfiGrpc.tla`) raffine la spec abstraite en ajoutant les mécanismes concrets
   (handles, callbacks, queues, shutdown).
4. La spec binding .NET (`DotNetBinding.tla`) raffine la spec FFI en modélisant roots, TCS,
   dispose.
5. Les safety properties (terminal unique, pas d'event tardif, pas de dereference stale) sont
   vérifiées sans fairness assumption.
6. Les liveness properties (progress vers terminal, progress vers `AK_RELEASED`) sont
   conditionnelles aux fairness déclarées.
7. Les propriétés de safety sont prouvées par TLAPS (TLA+ Proof System) plutôt que vérifiées
   par model checking TLC. TLC peut servir d'outil exploratoire mais la cible est la preuve
   formelle.

---

## Requirement 16 : Tests de performance comparatifs (équipe ArmoniK)

**User Story:** En tant que membre de l'équipe ArmoniK, je veux une campagne de tests comparatifs
entre le transport natif et le transport managé, afin de mesurer l'overhead introduit et décider
si des optimisations sont nécessaires.

### Acceptance Criteria

1. Un benchmark mesure la latence d'un unary call (P50, P95, P99) avec le transport natif vs
   managé.
2. Un benchmark mesure le throughput d'un streaming (messages/seconde) natif vs managé.
3. Un benchmark mesure l'overhead mémoire (allocations, RSS) natif vs managé.
4. Les benchmarks sont reproductibles en CI (même serveur, même charge, même réseau).
5. Les résultats sont documentés et servent de baseline pour les évolutions futures.
6. La campagne couvre au minimum .NET Framework 4.8 et .NET 8.

---

## Requirement 17 : Génération automatique des types de configuration (équipe ArmoniK)

**User Story:** En tant que membre de l'équipe ArmoniK, je veux que les types de configuration
côté .NET soient générés automatiquement à partir des types Rust, afin d'éliminer la maintenance
manuelle et le risque de drift entre les deux langages.

### Acceptance Criteria

1. Les types Rust de configuration (`TransportConfig`, `GrpcChannelConfig`, et leurs composants)
   sont la source de vérité unique pour les options du channel.
2. Un schéma JSON est généré automatiquement depuis ces types Rust (via `schemars` ou
   équivalent) et commité dans le dépôt.
3. Les types C# de configuration sont générés automatiquement à partir de ce schéma JSON.
4. L'ajout d'une option côté Rust se propage automatiquement au schéma JSON puis aux types C#
   sans intervention manuelle.
5. Un test vérifie que le schéma commité est à jour par rapport aux types Rust (le build échoue
   si le schéma est stale).
6. Les types C# générés sérialisent en JSON UTF-8 compatible avec le `serde::Deserialize` Rust.

---

## Hors périmètre V1

Les éléments suivants sont explicitement exclus de cette version :

- Télémétrie OpenTelemetry (logs, métriques, traces exportées depuis le natif)
- Hedging
- Load balancing générique et service config
- Compression gRPC configurable
- Interceptors génériques
- gRPC-Web
- Bindings Java, Python, C++
- Connectivity state API publique
- Rate limiting public configurable
- Retry policy par method name ou par call (V1 = default channel uniquement ; override par call
  et config par method name sont prévus juste après V1)
