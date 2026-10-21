#!/usr/bin/env bash
# Builds llama.cpp's llama-server for Android arm64 and puts it, with the libraries it loads, into
# the APK's jniLibs. Android only lets an app execute files from its native library folder, so the
# server ships named like a library (libllama-server.so). The CPU backend is built in every ARM
# variant and the best one for the phone is picked at run time.
#
#   NDK_HOME=... scripts/build-llama-android.sh          (LLAMA_TAG overrides the llama.cpp release)
set -euo pipefail

TAG=${LLAMA_TAG:-v0.7.0}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$ROOT/src-tauri/target/llama-android
OUT=$ROOT/src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a
NDK=${NDK_HOME:?set NDK_HOME to the Android NDK}

if [ ! -d "$WORK/src-$TAG" ]; then
  git clone --quiet --depth 1 --branch "$TAG" https://github.com/ggml-org/llama.cpp "$WORK/src-$TAG"
fi

cmake -S "$WORK/src-$TAG" -B "$WORK/build-$TAG" -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28 -DANDROID_STL=c++_shared \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON \
  -DGGML_NATIVE=OFF -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_OPENMP=OFF \
  -DLLAMA_CURL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_SERVER=ON
cmake --build "$WORK/build-$TAG" --target llama-server

# Staged first, so only these files are stripped (Tauri's own library shares the folder).
STAGE=$WORK/stage-$TAG
rm -rf "$STAGE" && mkdir -p "$STAGE" "$OUT"
cp "$WORK/build-$TAG/bin/llama-server" "$STAGE/libllama-server.so"
find "$WORK/build-$TAG/bin" -maxdepth 1 -name 'lib*.so' -exec cp {} "$STAGE/" \;
cp "$(find "$NDK/toolchains/llvm/prebuilt" -path '*aarch64-linux-android/libc++_shared.so' | head -1)" "$STAGE/"
"$(find "$NDK/toolchains/llvm/prebuilt" -name 'llvm-strip*' | head -1)" --strip-unneeded "$STAGE"/*.so
rm -f "$OUT"/libggml*.so "$OUT"/libllama-*.so "$OUT"/libllama.so "$OUT"/libmtmd*.so
cp "$STAGE"/*.so "$OUT/"
ls -la "$STAGE"
