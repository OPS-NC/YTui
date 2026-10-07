# ytui

> Inspired by [gaeldigard/youtube-tui](https://gitlab.com/gaeldigard/youtube-tui).

An audio-first YouTube client for the terminal, written in Rust to use as
little RAM, CPU and battery as possible.

A browser tab playing YouTube costs hundreds of megabytes. ytui idles at a
few: no runtime, no rendering engine, just a cell buffer for the screen.
yt-dlp (metadata) and curl (stream probing) run as throw-away processes, and a
single ffmpeg handles the audio output, the spectrum analyser and the video
clip at once. The release binaries embed ffmpeg and a JS engine, and keep
yt-dlp up to date on their own. The RAM of the whole process tree is shown in
the status bar.

## Install

### Homebrew (recommended — macOS and Linux)

```
brew install ops-nc/tap/ytui
ytui
```

The formula ([OPS-NC/homebrew-tap](https://github.com/OPS-NC/homebrew-tap))
installs the standalone binary and follows new releases on its own. Upgrade
with `brew update && brew upgrade ytui`.

### Linux without Homebrew

Grab the binary for your architecture (`x86_64` or `aarch64`, picked by
`uname -m`) from the latest release into `~/.local/bin`:

```
mkdir -p ~/.local/bin
wget -O ~/.local/bin/ytui "https://github.com/OPS-NC/YTui/releases/latest/download/ytui-linux-$(uname -m)"
chmod +x ~/.local/bin/ytui
ytui
```

Most distributions put `~/.local/bin` on the `PATH`; if `ytui` is not found,
add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile. Run the same
commands again to upgrade. `curl -fLo` works in place of `wget -O`.

### macOS without Homebrew

```
mkdir -p ~/.local/bin
curl -fLo ~/.local/bin/ytui "https://github.com/OPS-NC/YTui/releases/latest/download/ytui-macos-$(uname -m)"
chmod +x ~/.local/bin/ytui
```

`~/.local/bin` is not on macOS's default `PATH`: add
`export PATH="$HOME/.local/bin:$PATH"` to `~/.zshrc`. Downloaded this way the
binary is not quarantined; one fetched with a browser
or received over AirDrop needs `xattr -d com.apple.quarantine <file>` first
(it is not signed with an Apple developer account).

## Standalone binaries

Download them from the **Releases** page (`SHA256SUMS` lists their
checksums). They are built by the `build` GitHub Action on every `v*` tag, or
locally into `dist/` by `scripts/dist.sh`, and embed everything needed:

| File                  | For                                   |
|-----------------------|---------------------------------------|
| `ytui-macos-arm64`    | Apple Silicon Macs (M1 to M4)         |
| `ytui-macos-x86_64`   | Intel Macs                            |
| `ytui-linux-x86_64`   | Linux PCs (Intel / AMD)               |
| `ytui-linux-aarch64`  | Linux ARM (Raspberry Pi 4/5, servers) |

- **ffmpeg** and **quickjs** (yt-dlp's JS engine) are included and extracted
  once to `~/.local/share/ytui/bin`. This stripped-down ffmpeg takes ~12 MB of
  memory, against ~20 MB for a distribution build, so it is preferred over
  the system one.
- **yt-dlp** is downloaded on first launch (status bar: "installing
  yt-dlp…"), checked against its SHA-256, then updated automatically — at
  most one check a day. It cannot be frozen into the binary: YouTube changes
  its player every few weeks.

Still required from the system:

- `curl` (shipped with macOS and virtually every distribution);
- Linux: PulseAudio or PipeWire (Pulse layer), glibc ≥ 2.17 (every mainstream
  distribution; on Alpine/musl ytui falls back to the system's ffmpeg) and,
  for the managed yt-dlp, `python3` ≥ 3.10 or `unzip`.

The video clip needs a truecolor terminal (Ghostty, Kitty, WezTerm,
iTerm2…); Apple's Terminal is not one.

### Bringing your own tools

- a `yt-dlp` next to the binary takes precedence;
- `YTUI_YTDLP=/path/to/yt-dlp` (or `YTUI_YTDLP=yt-dlp` for the one on `PATH`)
  turns the managed copy off;
- an installed `deno`, `node` or `bun` is preferred over the embedded
  quickjs: it solves YouTube's challenges faster.

## Building

Requires a recent stable Rust, edition 2024 (https://rustup.rs).

```
./ytui.sh                # builds if needed, then runs
cargo build --release    # target/release/ytui
```

Without `bundle/`, the binary embeds nothing and uses the system's ffmpeg,
JS runtime and yt-dlp (or the managed yt-dlp) — install ffmpeg then
(`brew install ffmpeg`, `apt install ffmpeg`…).

Standalone binaries:

```
brew install nasm pkg-config cmake zig
cargo install --locked cargo-zigbuild
scripts/dist.sh          # builds ffmpeg + qjs once (bundle/), then dist/
```

`scripts/build-bundle.sh <target>` only rebuilds ffmpeg and qjs for one
target. The embedded ffmpeg is LGPL (v2.1+ on macOS, v3 on Linux because of
mbedTLS) with no GPL component; sources: https://ffmpeg.org/releases/.

Releasing: `git tag vX.Y && git push origin vX.Y` — the GitHub Action builds
the four binaries, tests them and publishes the release. The Homebrew tap
picks the new release up within 3 hours (or right away with
`gh workflow run update-ytui.yml --repo OPS-NC/homebrew-tap`).

## Usage

| Key / gesture          | Action                                       |
|------------------------|----------------------------------------------|
| `/`                    | Focus the search field                       |
| `↑` `↓`                | Search history (in the search field)         |
| `Tab` / `Shift+Tab`    | Next / previous panel                        |
| `Enter`                | Play the selection                           |
| double click           | Play the row (a single click only selects)   |
| `Space`                | Pause / resume                               |
| `←` `→`                | Back / forward 10 s                          |
| `n`                    | Next track                                   |
| `+` `-`                | Volume                                       |
| `v`                    | Video clip in place of the spectrum          |
| `V` / click the picture | Full-screen clip                            |
| `t`                    | Thumbnail grid (playlist mode)               |
| `T`                    | Theme picker (`default`, `dark`, `white`)    |
| `L`                    | Log in (browser cookies)                     |
| `Esc`                  | Back to the deck                             |
| `s` / `q`              | Stop / quit (`Ctrl+C` too)                   |

The footer keys are clickable too.

The search field also accepts a video URL or ID, or a playlist URL or ID.
History is kept in `~/.local/share/ytui/search_history` (`XDG_DATA_HOME` is
honoured).

The results column shows up during a search and hides when playing a
specific video or a playlist.

The "UP NEXT" list fills with YouTube's mix for the current track,
which plays on when it ends. A pasted playlist replaces it and plays in order
until "end of playlist"; playing a search result goes back to automatic
mode.

`v` shows the clip in place of the spectrum whenever the stream carries a
video track; otherwise the analyser stays. The picture is drawn with half
blocks (`▀`, top pixel as text colour, bottom pixel as background), hence the
truecolor requirement. The same ffmpeg process feeds it over a dedicated
pipe: going full screen does not reopen the stream. Unless the clip is asked
for, the video track is never decoded.

In playlist mode, `t` swaps the "UP NEXT" list for a grid of the whole
playlist, each thumbnail decoded by ffmpeg on the fly straight at its cell's
size (~1 kB each), only for what is on screen. Arrows move the selection
(highlighted by a high-contrast amber bar), the wheel or Home/End/PgUp/PgDn
scroll, Enter or a double click plays — a single click only selects. A
second `t` goes back to the text list.

## Themes

`T` opens the theme picker: `↑` `↓` preview each theme live on the whole
screen, `Enter` keeps it, `Esc` goes back. The choice is remembered in
`~/.local/share/ytui/theme`.

- **default** — a late-70s hi-fi separate: warm near-black, amber legends,
  an amber-to-ember meter.
- **dark** — cool graphite with white text, cyan legends, a teal → violet →
  pink meter.
- **white** — paper and ink: white panels, black text, ink-blue legends,
  burnt-orange accents.

## Authentication

By default ytui does not authenticate — public videos don't need it, and
reading a browser's cookies on every search would be surprising (a keychain
prompt, a locked database while the browser is open, extra latency). For
age-restricted, members-only or otherwise account-bound videos, press `L` in
the app: each press moves to the next browser (`firefox`, `chrome`,
`chromium`, `edge`, `brave`, `opera`, `vivaldi`, `safari`, `whale`), and one
last press goes back to "not logged in". The active state stays visible in
the status bar (`logged in (firefox)`) for as long as it is on.

Shortcut for a plain browser: `./ytui.sh --firefox` (or `--chrome`, `--edge`,
etc.) is the same as setting `YTUI_COOKIES_FROM_BROWSER` before launch.

For a specific profile or keyring (`chrome:Profile 1`, `firefox+kwallet`),
set `YTUI_COOKIES_FROM_BROWSER` before starting the app — same syntax as
yt-dlp's `--cookies-from-browser`. It is the starting point of the `L`
cycle, which then switches between the plain names above.

```
./ytui.sh --firefox
YTUI_COOKIES_FROM_BROWSER="chrome:Profile 1" ./ytui.sh
```

ytui then reads the cookies as the browser would, for yt-dlp's search,
playlist and suggestion requests and, only as a last resort, for stream
resolution (the anonymous strategies are still tried first).

When a browser is already set at launch, the results column shows the
YouTube home page ("Recommended for you") instead of staying empty until a
search. Running an actual search replaces those suggestions as usual;
logging in mid-session with `L` doesn't reload them — it is launch-time
behaviour only.

## Development

```
cargo build --release   # optimised binary (LTO, strip, panic=abort)
cargo test              # offline: parsing, layout, ffmpeg/qjs extraction
cargo test -- --ignored # managed yt-dlp install + https via ffmpeg (network, never YouTube)
cargo clippy
```

See [AGENTS.md](AGENTS.md) for the architecture and the project's rules.
