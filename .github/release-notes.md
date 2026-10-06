Client YouTube en TUI, audio seul, frugal en RAM. Binaires autonomes :
ffmpeg et le moteur JS (quickjs) sont embarqués, yt-dlp est installé puis
mis à jour automatiquement au premier lancement.

| Fichier | Pour |
|---|---|
| `ytui-macos-arm64` | Mac Apple Silicon (M1 à M4) |
| `ytui-macos-x86_64` | Mac Intel |
| `ytui-linux-x86_64` | Linux PC (Intel / AMD) |
| `ytui-linux-aarch64` | Linux ARM (Raspberry Pi 4/5, serveurs) |

```
chmod +x ytui-*
./ytui-linux-x86_64
```

macOS (binaire non signé par un compte développeur Apple) :

```
xattr -d com.apple.quarantine ytui-macos-arm64
```

Prérequis système : `curl` ; sous Linux, PulseAudio ou PipeWire, glibc ≥ 2.17,
et `python3` ≥ 3.10 ou `unzip` pour installer yt-dlp. Le clip vidéo demande
un terminal truecolor (Ghostty, Kitty, WezTerm, iTerm2…).

`SHA256SUMS` donne l'empreinte de chaque binaire.
