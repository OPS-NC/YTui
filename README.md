# ytui

Client YouTube en TUI, audio seul.

## Prérequis

- Python >= 3.10 et python3-venv
- ffmpeg
- Linux/WSL : PulseAudio ou PipeWire (couche Pulse)

Debian / Ubuntu / WSL :

```
sudo apt install -y python3-venv ffmpeg
```

Arch :

```
sudo pacman -S python ffmpeg
```

Fedora :

```
sudo dnf install python3 ffmpeg
```

macOS :

```
brew install python ffmpeg
```

## Usage

| Touche / geste     | Action                                    |
|--------------------|-------------------------------------------|
| `/`                | Focus recherche                           |
| `↑` `↓`            | Historique des recherches (champ actif)   |
| `Entrée`           | Lire la sélection                         |
| `Espace`           | Pause / reprise                           |
| `←` `→`            | Reculer / avancer de 10 s                 |
| `n`                | Piste suivante                            |
| `+` `-`            | Volume                                    |
| `t`                | Grille de miniatures (mode playlist)      |
| clic sur l'image   | Clip en plein écran                       |
| `v`                | Idem au clavier                           |
| `Échap`            | Revenir à la platine                      |
| `s` / `q`          | Arrêt / quitter                           |

La colonne des résultats apparaît pendant une recherche et se masque lors de
la lecture d'une vidéo précise ou d'une playlist.

La platine affiche le clip à la place du spectre dès que le flux porte une
piste vidéo ; sinon l'analyseur reste à l'écran. L'image est rendue en
demi-blocs (`▀`, pixel du haut en couleur de texte, pixel du bas en fond),
donc un terminal truecolor est nécessaire. La même instance de ffmpeg s'en
charge sur un tube dédié : passer en plein écran ne réouvre pas le flux.

En lecture de playlist, `t` remplace la liste « SUITE » par une grille de
toute la playlist, chaque miniature décodée par ffmpeg à la volée (le même
rendu demi-blocs que le clip, uniquement pour ce qui est visible à l'écran).
Flèches pour déplacer la sélection (surlignée d'une barre ambrée bien
contrastée), molette ou Origine/Fin/Page préc./Page suiv. pour faire défiler,
Entrée ou double-clic pour lire — un simple clic ne fait que sélectionner.
Un second `t` revient à la liste texte.

## Lancement

```
./ytui.sh
```

Le script crée `.venv`, installe les dépendances Python au premier appel, puis
démarre l'application.

Installation manuelle équivalente :

```
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
.venv/bin/python -m ytui
```
