# ytui

Client YouTube en TUI, audio seul, écrit en Rust pour consommer le moins de
RAM, de CPU et de batterie possible.

Un onglet de navigateur qui lit YouTube coûte des centaines de mégaoctets.
ytui garde au repos quelques mégaoctets : pas de runtime, pas de moteur de
rendu, un tampon de cellules pour l'écran. yt-dlp (métadonnées) et curl
(vérification du flux) sont lancés comme des processus jetables, et un seul
ffmpeg assure à la fois la sortie son, l'analyseur et le clip vidéo. La RAM
de tout l'arbre de processus est affichée dans la barre du bas.

## Prérequis

- ffmpeg
- yt-dlp
- curl (présent par défaut sur macOS et la plupart des distributions)
- conseillé : un moteur JS pour yt-dlp (`deno`, `node`, `bun` ou `quickjs`)
- Linux/WSL : PulseAudio ou PipeWire (couche Pulse)
- pour compiler : Rust stable récent, édition 2024 (`rustup`, https://rustup.rs)

Debian / Ubuntu / WSL :

```
sudo apt install -y ffmpeg curl yt-dlp
```

Arch :

```
sudo pacman -S ffmpeg curl yt-dlp
```

Fedora :

```
sudo dnf install ffmpeg curl yt-dlp
```

macOS :

```
brew install ffmpeg yt-dlp
```

Le paquet yt-dlp des distributions est souvent en retard sur YouTube ;
`pipx install yt-dlp` donne la dernière version.

## Lancement

```
./ytui.sh
```

Le script compile le binaire en mode release au premier appel (puis seulement
si les sources ont changé) et démarre l'application.

Équivalent manuel :

```
cargo build --release
./target/release/ytui
```

Le binaire est autonome : `target/release/ytui` peut être copié n'importe où
dans le `PATH`. Un `yt-dlp` placé à côté de lui est utilisé en priorité.

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
cargo test              # tests hors-ligne : parsing d'URL, sortie yt-dlp, mise en page
cargo clippy
```

Voir [AGENTS.md](AGENTS.md) pour l'architecture et les règles du projet.
