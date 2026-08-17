# ytui

Client YouTube en TUI, audio seul. Pas de rendu vidéo, pas de navigateur.
Recherche, lecture, visualiseur spectral, enchaînement automatique sur les
suggestions. Environ 95 Mo de RSS en lecture, processus ffmpeg compris.

## Prérequis système

- Python >= 3.10
- ffmpeg (décodage + sortie audio)
- Linux/WSL : PulseAudio ou PipeWire avec la couche de compatibilité Pulse
- macOS : rien de plus, la sortie passe par AudioToolbox / CoreAudio
- python3-venv

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

Sous WSL, WSLg fournit déjà le serveur Pulse (`PULSE_SERVER=unix:/mnt/wslg/PulseServer`),
rien à configurer.

La sortie audio est choisie selon la plateforme : `-f pulse` sur Linux,
`-f audiotoolbox` sur macOS. Rien à configurer.

### macOS : certificats CA

Avec un Python installé depuis python.org, aucun magasin de certificats n'est
fourni et toute recherche échoue (`CERTIFICATE_VERIFY_FAILED`). `certifi` fait
partie des dépendances, ce qui suffit à yt-dlp ; sinon lancez une fois
`/Applications/Python 3.x/Install Certificates.command`.

## Flux, 403 et qualité

Depuis 2025 YouTube exige un *PO token* pour la plupart de ses clients. Les
clients qui s'en passent (`android_vr`) donnent bien des formats audio seuls,
mais leurs URLs refusent les requêtes de plage ouverte : ffmpeg ouvre un flux
avec `Range: bytes=0-` et reçoit `403` à chaque essai — d'où les échecs en
boucle. Mesuré : au-delà de ~512 Kio par requête, `403` systématique, et
yt-dlp lui-même échoue sur ces formats.

ytui teste donc chaque URL avec exactement la requête que ffmpeg enverra, puis
retient la première qui répond :

1. `audio seul` — DASH opus 135 kb/s, aucun octet de vidéo ;
2. `flux muxé 360p` — le progressif du client `android` (~300 kb/s, AAC ~96 kb/s),
   dont ffmpeg jette la vidéo. Moins bon, mais il répond.

La stratégie retenue est réutilisée pour les pistes suivantes, et la platine
affiche laquelle est en cours. Une URL non validée n'est jamais confiée au
lecteur, ce qui supprime la boucle de tentatives perdues.

### Retrouver l'audio seul en permanence

Il faut fournir un PO token, ce que font tous les clients YouTube alternatifs
aujourd'hui. Un fournisseur local suffit :

```
.venv/bin/pip install -U bgutil-ytdlp-pot-provider
docker run --name bgutil-provider -d --init brainicism/bgutil-ytdlp-pot-provider
```

yt-dlp détecte le greffon tout seul ; sans Docker, le dépôt du fournisseur
tourne aussi en mode script avec Node >= 20. Un moteur JS (`brew install deno`)
est par ailleurs recommandé : yt-dlp en a besoin pour les défis de signature et
l'extraction sans moteur est dépréciée. `--cookies-from-browser` est une autre
voie, au prix d'attacher son compte YouTube aux requêtes.

## Lancement

```
./ytui.sh
```

Le script crée `.venv` et installe les dépendances Python (`textual`, `yt-dlp`,
`certifi`) au premier appel, puis démarre l'application.

Installation manuelle équivalente :

```
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
.venv/bin/python -m ytui
```

## Usage

Le champ de recherche accepte une requête libre, une URL YouTube
(`watch`, `youtu.be`, `shorts`, `embed`) ou un ID de vidéo brut.

| Touche      | Action                  |
|-------------|-------------------------|
| `/`         | Focus recherche         |
| `Entrée`    | Lire la sélection       |
| `Espace`    | Pause / reprise         |
| `gauche` `droite` | Reculer / avancer de 10 s |
| `n`         | Piste suivante          |
| `+` `-`     | Volume                  |
| `s`         | Arrêt                   |
| `q`         | Quitter                 |

Souris : simple clic pour sélectionner, double clic pour lire.

## Notes

`yt-dlp` est appelé comme binaire externe, jamais importé : la mémoire de
l'extraction est rendue à l'OS après chaque requête. Un unique ffmpeg décode,
sort sur PulseAudio et alimente le visualiseur.

Les URLs signées de googlevideo sont fréquemment refusées (403) dès la première
requête ; le lecteur réessaie automatiquement avec une URL fraîche.
