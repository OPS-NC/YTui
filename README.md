# ytui

Client YouTube en TUI, audio seul, écrit en Rust pour consommer le moins de
RAM, de CPU et de batterie possible.

Un onglet de navigateur qui lit YouTube coûte des centaines de mégaoctets.
ytui garde au repos quelques mégaoctets : pas de runtime, pas de moteur de
rendu, un tampon de cellules pour l'écran. yt-dlp (métadonnées) et curl
(vérification du flux) sont lancés comme des processus jetables, et un seul
ffmpeg assure à la fois la sortie son, l'analyseur et le clip vidéo. Le
binaire distribué embarque ffmpeg et un moteur JS, et maintient yt-dlp à jour
tout seul. La RAM
de tout l'arbre de processus est affichée dans la barre du bas.

## Binaire autonome

Les binaires de `dist/` embarquent tout ce qu'il faut :

| Fichier               | Pour                                   |
|-----------------------|----------------------------------------|
| `ytui-macos-arm64`    | Mac Apple Silicon (M1 à M4)            |
| `ytui-linux-x86_64`   | Linux PC (Intel / AMD)                 |
| `ytui-linux-aarch64`  | Linux ARM (Raspberry Pi 4/5, serveurs) |

- **ffmpeg** et **quickjs** (moteur JS de yt-dlp) sont inclus, et extraits
  une fois dans `~/.local/share/ytui/bin`. Ce ffmpeg réduit au strict
  nécessaire pèse ~12 Mo en mémoire, contre ~20 Mo pour un ffmpeg de
  distribution : il est utilisé de préférence à celui du système.
- **yt-dlp** est téléchargé au premier lancement (barre d'état : « installation
  de yt-dlp… »), vérifié par SHA-256, puis mis à jour automatiquement — un
  contrôle par jour au plus. Il ne peut pas être figé dans le binaire :
  YouTube change son lecteur toutes les quelques semaines.

Il reste à fournir, côté système :

- `curl` (présent par défaut sur macOS et quasiment toutes les distributions) ;
- Linux : PulseAudio ou PipeWire (couche Pulse), une glibc ≥ 2.17 (toutes les
  distributions courantes ; sur Alpine/musl, ytui se rabat sur le ffmpeg du
  système) et, pour la version de yt-dlp gérée, `python3` ≥ 3.10 ou `unzip`.

macOS : un binaire reçu par AirDrop ou téléchargé est bloqué par Gatekeeper
(il n'est pas signé par un compte développeur Apple) :

```
xattr -d com.apple.quarantine ytui-macos-arm64
chmod +x ytui-macos-arm64 && ./ytui-macos-arm64
```

Le clip vidéo demande un terminal truecolor (iTerm2, Ghostty, WezTerm,
Kitty…), le Terminal d'Apple ne l'est pas.

### Choisir ses propres outils

- un `yt-dlp` placé à côté du binaire est prioritaire ;
- `YTUI_YTDLP=/chemin/vers/yt-dlp` (ou `YTUI_YTDLP=yt-dlp` pour celui du
  `PATH`) désactive la copie gérée ;
- un `deno`, `node` ou `bun` installé est préféré au quickjs embarqué : il
  résout les challenges YouTube plus vite.

## Compiler

Prérequis : Rust stable récent, édition 2024 (https://rustup.rs).

```
./ytui.sh                # compile si besoin puis lance
cargo build --release    # target/release/ytui
```

Sans `bundle/`, le binaire n'embarque rien et utilise le ffmpeg, le moteur JS
et le yt-dlp du système (ou la copie gérée de yt-dlp) — il faut alors
installer ffmpeg (`brew install ffmpeg`, `apt install ffmpeg`…).

Binaires autonomes :

```
brew install nasm pkg-config cmake zig
cargo install --locked cargo-zigbuild
scripts/dist.sh          # compile ffmpeg + qjs une fois (bundle/), puis dist/
```

`scripts/build-bundle.sh <cible>` ne reconstruit que ffmpeg et qjs pour une
cible. Le ffmpeg embarqué est sous LGPL (v2.1+ sur macOS, v3 sur Linux à
cause de mbedTLS), sans composant GPL ; sources : https://ffmpeg.org/releases/.

## Usage

| Touche / geste     | Action                                    |
|--------------------|-------------------------------------------|
| `/`                | Focus recherche                           |
| `↑` `↓`            | Historique des recherches (champ actif)   |
| `Tab` / `Maj+Tab`  | Panneau suivant / précédent               |
| `Entrée`           | Lire la sélection                         |
| double-clic        | Lire la ligne (un clic ne fait que sélectionner) |
| `Espace`           | Pause / reprise                           |
| `←` `→`            | Reculer / avancer de 10 s                 |
| `n`                | Piste suivante                            |
| `+` `-`            | Volume                                    |
| `v`                | Clip à la place du spectre                |
| `V` / clic sur l'image | Clip en plein écran                   |
| `t`                | Grille de miniatures (mode playlist)      |
| `L`                | Connexion (cookies d'un navigateur)       |
| `Échap`            | Revenir à la platine                      |
| `s` / `q`          | Arrêt / quitter (`Ctrl+C` aussi)          |

Les touches du pied de page sont aussi cliquables.

La recherche accepte aussi une URL ou un ID de vidéo, ou une URL / un ID de
playlist. L'historique est conservé dans
`~/.local/share/ytui/search_history` (`XDG_DATA_HOME` respecté).

La colonne des résultats apparaît pendant une recherche et se masque lors de
la lecture d'une vidéo précise ou d'une playlist.

La liste « SUITE » se remplit du mix YouTube de la piste en cours, qui
s'enchaîne en fin de lecture. Une playlist collée la remplace et la lit dans
l'ordre jusqu'à « fin de la playlist » ; lire un résultat de recherche revient
au mode automatique.

`v` affiche le clip à la place du spectre dès que le flux porte une piste
vidéo ; sinon l'analyseur reste à l'écran. L'image est rendue en demi-blocs
(`▀`, pixel du haut en couleur de texte, pixel du bas en fond), donc un
terminal truecolor est nécessaire. La même instance de ffmpeg s'en charge sur
un tube dédié : passer en plein écran ne réouvre pas le flux. Sans clip
demandé, la piste vidéo n'est jamais décodée.

En lecture de playlist, `t` remplace la liste « SUITE » par une grille de
toute la playlist, chaque miniature décodée par ffmpeg à la volée directement
à la taille de sa cellule (~1 ko chacune), uniquement pour ce qui est visible
à l'écran. Flèches pour déplacer la sélection (surlignée d'une barre ambrée
bien contrastée), molette ou Origine/Fin/Page préc./Page suiv. pour faire
défiler, Entrée ou double-clic pour lire — un simple clic ne fait que
sélectionner. Un second `t` revient à la liste texte.

## Authentification

Par défaut ytui ne s'authentifie pas — les vidéos publiques n'en ont pas
besoin, et lire les cookies d'un navigateur à chaque recherche serait
surprenant (invite du trousseau, base verrouillée si le navigateur est
ouvert, latence en plus). Pour les vidéos limitées par âge, réservées aux
membres ou autrement liées à un compte, appuyez sur `L` dans l'app : chaque
appui passe au navigateur suivant (`firefox`, `chrome`, `chromium`, `edge`,
`brave`, `opera`, `vivaldi`, `safari`, `whale`), et un dernier appui revient à
« pas connecté ». L'état actif reste affiché dans la barre du bas
(`connecté (firefox)`) tant qu'il l'est.

Raccourci pour un navigateur simple : `./ytui.sh --firefox` (ou `--chrome`,
`--edge`, etc.) équivaut à positionner `YTUI_COOKIES_FROM_BROWSER` avant le
lancement.

Pour un profil ou un trousseau précis (`chrome:Profile 1`, `firefox+kwallet`),
positionnez `YTUI_COOKIES_FROM_BROWSER` avant de lancer l'app — même syntaxe
que l'option `--cookies-from-browser` de yt-dlp. C'est le point de départ du
cycle de `L`, qui bascule ensuite sur les noms simples ci-dessus.

```
./ytui.sh --firefox
YTUI_COOKIES_FROM_BROWSER="chrome:Profile 1" ./ytui.sh
```

ytui lit alors les cookies comme le ferait le navigateur, pour les requêtes
yt-dlp de recherche, playlist, suggestions et, en dernier recours seulement,
de résolution du flux (les stratégies anonymes restent essayées d'abord).

Si un navigateur est déjà positionné au lancement, la colonne des résultats
affiche directement la page d'accueil YouTube (« Recommandé pour vous ») au
lieu de rester vide en attendant une recherche. Lancer une vraie recherche
remplace ces suggestions normalement ; se connecter en cours de session avec
`L` ne les recharge pas — c'est uniquement le comportement au démarrage.

## Développement

```
cargo build --release   # binaire optimisé (LTO, strip, panic=abort)
cargo test              # hors-ligne : parsing, mise en page, extraction ffmpeg/qjs
cargo test -- --ignored # installation de yt-dlp (télécharge depuis GitHub)
cargo clippy
```

Voir [AGENTS.md](AGENTS.md) pour l'architecture et les règles du projet.
