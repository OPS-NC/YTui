#!/usr/bin/env bash
# Builds the binaries ytui embeds: a minimal ffmpeg and quickjs-ng's `qjs`,
# for one Rust target, into bundle/<target>/. `cargo build` then embeds
# whatever it finds there (see build.rs); without it ytui relies on the
# system's ffmpeg / JS runtime as before.
#
#   scripts/build-bundle.sh aarch64-apple-darwin
#   scripts/build-bundle.sh x86_64-apple-darwin     (cross-built from arm64 too)
#   scripts/build-bundle.sh x86_64-unknown-linux-musl
#   scripts/build-bundle.sh aarch64-unknown-linux-musl
#
# Needs: curl, make, cmake, pkg-config, nasm (x86_64), zig (Linux targets).
#
# ffmpeg is cut down to what ytui actually runs: https input (mp4/webm/HLS,
# jpeg thumbnails), aac/opus/vorbis/mp3/h264/mjpeg decoding, the volume /
# aresample / scale / fps filters, s16le + rawvideo pipes and the OS sink.
# It is LGPL, no GPL component (v2.1+ on macOS; v3 on Linux, required by
# mbedTLS's Apache-2.0 licence) — sources: https://ffmpeg.org/releases/.
#
# Linux: ffmpeg links glibc 2.17 and the system's libpulse.so.0 dynamically
# (PulseAudio's client library cannot be linked statically, and every
# Pulse/PipeWire desktop ships it); everything else is static. qjs is a
# fully static musl binary.
set -euo pipefail

TARGET=${1:?usage: $0 <rust-target>}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SRC=$ROOT/target/bundle-src
WORK=$ROOT/target/bundle-build/$TARGET
OUT=$ROOT/bundle/$TARGET
JOBS=$(sysctl -n hw.ncpu 2>/dev/null || nproc)

FFMPEG_V=9.0.2
MBEDTLS_V=3.6.5
QJS_V=0.17.0
PULSE_V=17.0

mkdir -p "$SRC" "$WORK" "$OUT"

fetch() { # url dest-dir
    local name; name=$(basename "$1")
    [ -f "$SRC/$name" ] || curl -fsSL -o "$SRC/$name" "$1"
    [ -d "$SRC/$2" ] || tar -xf "$SRC/$name" -C "$SRC"
}

case $TARGET in
    aarch64-apple-darwin) OS=macos; ARCH=aarch64; MAC_ARCH=arm64 ;;
    x86_64-apple-darwin) OS=macos; ARCH=x86_64; MAC_ARCH=x86_64 ;;
    x86_64-unknown-linux-musl) OS=linux; ARCH=x86_64 ;;
    aarch64-unknown-linux-musl) OS=linux; ARCH=aarch64 ;;
    *) echo "cible non prise en charge : $TARGET" >&2; exit 1 ;;
esac

# ------------------------------------------------------------- toolchain
if [ "$OS" = linux ]; then
    # Wrappers so that configure/cmake see one plain executable per tool.
    TC=$WORK/tc; mkdir -p "$TC"
    for tool in cc:cc c++:c++ ar:ar ranlib:ranlib; do
        name=${tool%%:*}; sub=${tool##*:}
        case $sub in
            cc|c++) printf '#!/bin/sh\nexec zig %s -target "$ZIG_TARGET" "$@"\n' "$sub" ;;
            *) printf '#!/bin/sh\nexec zig %s "$@"\n' "$sub" ;;
        esac > "$TC/$name"
        chmod +x "$TC/$name"
    done
    GLIBC_TARGET=$ARCH-linux-gnu.2.17
    MUSL_TARGET=$ARCH-linux-musl
fi

# ------------------------------------------------------------- quickjs-ng
build_qjs() {
    fetch "https://github.com/quickjs-ng/quickjs/archive/refs/tags/v$QJS_V.tar.gz" "quickjs-$QJS_V"
    local b=$WORK/qjs
    local args=(-S "$SRC/quickjs-$QJS_V" -B "$b" -DCMAKE_BUILD_TYPE=MinSizeRel -DBUILD_SHARED_LIBS=OFF)
    if [ "$OS" = macos ]; then
        args+=(-DCMAKE_OSX_ARCHITECTURES="$MAC_ARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0)
    fi
    if [ "$OS" = linux ]; then
        export ZIG_TARGET=$MUSL_TARGET
        args+=(-DCMAKE_C_COMPILER="$TC/cc" -DCMAKE_AR="$TC/ar" -DCMAKE_RANLIB="$TC/ranlib"
               -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR="$ARCH"
               -DQJS_BUILD_CLI_STATIC=ON -DCMAKE_EXE_LINKER_FLAGS=-s)
    fi
    cmake "${args[@]}" >/dev/null
    cmake --build "$b" --target qjs_exe -j "$JOBS" >/dev/null
    cp "$b/qjs" "$OUT/qjs"
    strip_bin "$OUT/qjs"
}

strip_bin() {
    if [ "$OS" = macos ]; then strip -x "$1"; else
        :   # Linux binaries are linked with -s already
    fi
}

# ---------------------------------------------------- Linux-only helpers
build_mbedtls() {
    fetch "https://github.com/Mbed-TLS/mbedtls/releases/download/mbedtls-$MBEDTLS_V/mbedtls-$MBEDTLS_V.tar.bz2" "mbedtls-$MBEDTLS_V"
    local b=$WORK/mbedtls
    export ZIG_TARGET=$GLIBC_TARGET
    cmake -S "$SRC/mbedtls-$MBEDTLS_V" -B "$b" -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_C_COMPILER="$TC/cc" -DCMAKE_AR="$TC/ar" -DCMAKE_RANLIB="$TC/ranlib" \
        -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR="$ARCH" \
        -DENABLE_PROGRAMS=OFF -DENABLE_TESTING=OFF -DUSE_SHARED_MBEDTLS_LIBRARY=OFF \
        -DCMAKE_INSTALL_PREFIX="$WORK/prefix" >/dev/null
    cmake --build "$b" -j "$JOBS" >/dev/null
    cmake --install "$b" >/dev/null
}

# libpulse at link time only: the real headers, and a stub libpulse.so.0
# exporting every pa_* function they declare. At run time the system's
# libpulse.so.0 is loaded in its place (same soname).
build_pulse_stub() {
    fetch "https://freedesktop.org/software/pulseaudio/releases/pulseaudio-$PULSE_V.tar.xz" "pulseaudio-$PULSE_V"
    local inc=$WORK/prefix/include/pulse lib=$WORK/prefix/lib
    mkdir -p "$inc" "$lib/pkgconfig"
    cp "$SRC/pulseaudio-$PULSE_V"/src/pulse/*.h "$inc/"
    local major=${PULSE_V%%.*} minor=${PULSE_V#*.}
    sed -e "s/@PA_MAJORMINOR@/$PULSE_V/; s/@PA_API_VERSION@/12/; s/@PA_PROTOCOL_VERSION@/35/" \
        -e "s/@PA_MAJOR@/$major/; s/@PA_MINOR@/$minor/; s/@PA_MICRO@/0/" \
        "$SRC/pulseaudio-$PULSE_V/src/pulse/version.h.in" > "$inc/version.h"
    cat "$inc"/*.h | tr '\n' ' ' \
        | grep -oE '[a-z_0-9 *]+ \**pa_[a-z0-9_]+ *\(' \
        | grep -oE 'pa_[a-z0-9_]+ *\($' | tr -d ' (' | sort -u \
        | awk '{print "void " $1 "(void) {}"}' > "$WORK/pulse_stub.c"
    export ZIG_TARGET=$GLIBC_TARGET
    "$TC/cc" -shared -fPIC -w -Wl,-soname,libpulse.so.0 -o "$lib/libpulse.so" "$WORK/pulse_stub.c"
    cat > "$lib/pkgconfig/libpulse.pc" <<PC
prefix=$WORK/prefix
Name: libpulse
Description: stub for linking
Version: $PULSE_V
Libs: -L\${prefix}/lib -lpulse
Cflags: -I\${prefix}/include
PC
}

# ------------------------------------------------------------- ffmpeg
build_ffmpeg() {
    fetch "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_V.tar.xz" "ffmpeg-$FFMPEG_V"
    local b=$WORK/ffmpeg; mkdir -p "$b"
    local common=(
        --disable-everything --disable-autodetect --disable-doc --disable-debug
        --disable-programs --enable-ffmpeg --enable-static --disable-shared
        --enable-small --enable-network --enable-pthreads
        --enable-avformat --enable-avcodec --enable-avfilter --enable-avdevice
        --enable-swresample --enable-swscale
        --enable-protocol=file,pipe,http,https,httpproxy,tcp,tls,hls,crypto
        --enable-demuxer=mov,matroska,hls,mpegts,aac,mp3,ogg,wav,image_jpeg_pipe,image2
        --enable-decoder=aac,opus,vorbis,mp3,mp3float,h264,mjpeg,pcm_s16le
        --enable-parser=aac,opus,vorbis,h264,mpegaudio,mjpeg
        --enable-encoder=pcm_s16le,rawvideo
        --enable-muxer=pcm_s16le,rawvideo
        --enable-filter=abuffer,abuffersink,buffer,buffersink,aresample,aformat,anull,volume,format,null,scale,fps,trim,atrim
    )
    local extra=()
    if [ "$OS" = macos ]; then
        extra=(--enable-securetransport --enable-audiotoolbox --enable-outdev=audiotoolbox
               --arch="$ARCH" --cc="clang -arch $MAC_ARCH"
               --extra-cflags=-mmacosx-version-min=12.0 --extra-ldflags=-mmacosx-version-min=12.0)
        # Intel binaries are cross-built on Apple Silicon (no Intel runners).
        [ "$(uname -m)" = "$MAC_ARCH" ] || extra+=(--enable-cross-compile --target-os=darwin)
    else
        build_mbedtls
        build_pulse_stub
        export ZIG_TARGET=$GLIBC_TARGET
        extra=(--enable-cross-compile --target-os=linux --arch="$ARCH"
               --cc="$TC/cc" --cxx="$TC/c++" --ar="$TC/ar" --ranlib="$TC/ranlib"
               --strip=true
               --pkg-config="$(command -v pkg-config)" --pkg-config-flags=--static
               --enable-version3 --enable-mbedtls --enable-libpulse --enable-outdev=pulse
               --extra-cflags="-I$WORK/prefix/include" --extra-ldflags="-L$WORK/prefix/lib" --extra-ldexeflags=-s)
        export PKG_CONFIG_PATH=$WORK/prefix/lib/pkgconfig:$WORK/prefix/lib64/pkgconfig
        export PKG_CONFIG_LIBDIR=$PKG_CONFIG_PATH
    fi
    (cd "$b" && "$SRC/ffmpeg-$FFMPEG_V/configure" "${common[@]}" "${extra[@]}" >configure.log 2>&1) \
        || { tail -30 "$b/ffbuild/config.log"; exit 1; }
    if [ "$OS" = linux ]; then
        # configure finds sysctl() in glibc 2.17's symbol list, but zig ships
        # current glibc headers, where <sys/sysctl.h> no longer exists.
        sed -i.bak 's/#define HAVE_SYSCTL 1/#define HAVE_SYSCTL 0/' "$b/config.h"
        [ -f "$b/config.asm" ] && sed -i.bak 's/HAVE_SYSCTL 1/HAVE_SYSCTL 0/' "$b/config.asm"
    fi
    make -C "$b" -j "$JOBS" ffmpeg >"$b/make.log" 2>&1 || { tail -30 "$b/make.log"; exit 1; }
    # Linux: the Makefile's strip step is disabled (--strip=true) since the
    # link already ran with -s, so ffmpeg_g is the final binary.
    if [ "$OS" = linux ]; then cp "$b/ffmpeg_g" "$OUT/ffmpeg"; else cp "$b/ffmpeg" "$OUT/ffmpeg"; fi
    strip_bin "$OUT/ffmpeg"
}

build_qjs
build_ffmpeg
ls -l "$OUT"
