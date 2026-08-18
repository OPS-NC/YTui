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
