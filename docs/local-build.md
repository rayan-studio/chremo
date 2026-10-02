# Caddy : build local de cette version (Windows x64)

Depuis PowerShell, à la racine du dépôt :

```powershell
.\caddy build                      # Compiler en medium, cache existant réutilisé
.\caddy check                      # Vérifier le Rust sans édition de liens finale
.\caddy run https://example.org     # Lancer immédiatement le dernier exécutable
.\caddy run --build https://example.org # Compiler puis lancer si le build réussit
.\caddy build --profile fast       # Profil de développement peu optimisé
.\caddy build --profile release    # Build optimisé
.\caddy build -j 4                 # Limiter les compilations simultanées
.\caddy build --offline            # Utiliser uniquement les dépendances déjà téléchargées
.\caddy build --dry-run            # Afficher la commande après préparation de l'environnement
.\caddy doctor                     # Localiser les outils nécessaires
```

PowerShell demande `./` ou `.\` pour les commandes du dossier courant (ici `./caddy`).
Pour écrire simplement `caddy build` et `caddy run` pendant cette session :

```powershell
$env:PATH = "$PWD;$env:PATH"
caddy build
caddy run
```

Cela donne priorité au Caddy de ce dépôt si un autre programme porte déjà ce nom.
PowerShell utilise `caddy.ps1`, qui transmet les arguments directement à Python.
`cmd.exe` utilise `caddy.cmd`. Mettre les URL contenant des caractères spéciaux
entre guillemets. Pour séparer explicitement les options de Caddy de celles du
navigateur dans PowerShell, écrire le séparateur entre guillemets :
`caddy run '--' --version`. Un `--` non cité est consommé par PowerShell ; Caddy
accepte aussi directement une URL suivie des options du navigateur.

Le lanceur appelle Cargo directement, sans chargement de `mach`, synchronisation
`uv` ou bootstrap à chaque invocation. Il utilise Python de `.venv` lorsqu'il
existe, sinon `python` du PATH. Le pilote n'utilise que la bibliothèque standard
Python ; les scripts de compilation des dépendances gardent leurs propres prérequis.
Rust utilise toujours la version de `rust-toolchain.toml`.

L'environnement Visual Studio x64 est chargé automatiquement, avec LLVM, CMake
et NASM. Les DLL ANGLE et MSVC sont placées près de l'exécutable après un build
réussi ; les fichiers identiques ne sont pas réécrits. `run` transmet les arguments
au navigateur et conserve son code de sortie. Par défaut, il lance le dernier
exécutable **sans appeler Cargo, Visual Studio ni scanner les DLL**. Les changements
de code ne sont donc visibles qu'après `caddy build`, ou avec `caddy run --build`.
Si l'exécutable manque, Caddy indique la commande de build à utiliser.

Le profil par défaut est `medium`, identique au profil de l'exécutable présent
lors de la création de cet outil. Le dossier `target` est conservé, ainsi que
`CARGO_TARGET_DIR` s'il est défini. Cargo décide quels fichiers recompiler.
`--locked` empêche une mise à jour implicite de `Cargo.lock`.

Le profil `fast` conserve la compilation incrémentale et les assertions du profil
`dev`, avec moins d'informations de débogage. Il conserve aussi les réglages
spécifiques aux dépendances, notamment celui de `servo-script` pour Windows.
Il produit `target/fast/servoshell.exe`. **Son premier build remplit un nouveau
cache** et peut donc être long. Le navigateur sera moins optimisé qu'en `medium`.
Rester sur le même profil, les mêmes features et les mêmes flags évite les
invalidations de cache. `sccache` est utilisé s'il est déjà disponible.

Le périmètre est le build natif Windows x64 de ce fork, avec les features Cargo
par défaut et le backend média `dummy`, comme le build `medium` existant : cet
outil n'active donc pas GStreamer. Il ne remplace pas les commandes de tests,
de distribution ou de compilation Android de `mach`. Les outils et dépendances
natifs doivent être installés au préalable. Une configuration `.servobuild`
provoque une erreur explicite, car ses réglages ne sont pas interprétés.

Les durées réelles de `build` dépendent surtout des compilations Rust/C++ et de
l'édition de liens. Même sans recompilation, la vérification du cache Cargo peut
prendre plusieurs dizaines de secondes ; `run` évite entièrement cette étape.
Le lanceur affiche le temps du build et Cargo écrit son rapport dans
`target/cargo-timings/cargo-timing.html` (ou sous `CARGO_TARGET_DIR`). Aucun facteur
d'accélération n'est garanti. Un build après modification et un build sans
modification permettent de vérifier l'efficacité du cache.

Tests du pilote, sans recompiler Servo :

```powershell
.venv\Scripts\python.exe -m unittest discover -s support -p test_local_build.py
```
