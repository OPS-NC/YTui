Audio-first YouTube client for the terminal, light on RAM. Standalone
binaries: ffmpeg and a JS engine (quickjs) are embedded, yt-dlp is installed
and kept up to date automatically on first launch.

| File | For |
|---|---|
| `ytui-macos-arm64` | Apple Silicon Macs (M1 to M4) |
| `ytui-macos-x86_64` | Intel Macs |
| `ytui-linux-x86_64` | Linux PCs (Intel / AMD) |
| `ytui-linux-aarch64` | Linux ARM (Raspberry Pi 4/5, servers) |

```
chmod +x ytui-*
./ytui-linux-x86_64
```

macOS (the binary is not signed with an Apple developer account):

```
xattr -d com.apple.quarantine ytui-macos-arm64
```

System requirements: `curl`; on Linux, PulseAudio or PipeWire, glibc ≥ 2.17,
and `python3` ≥ 3.10 or `unzip` to install yt-dlp. The video clip needs a
truecolor terminal (Ghostty, Kitty, WezTerm, iTerm2…).

`SHA256SUMS` lists each binary's checksum.
