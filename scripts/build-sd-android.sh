#!/usr/bin/env bash
# Builds stable-diffusion.cpp's sd-server for Android arm64 and puts it into the APK's jniLibs as
# libsd-server.so (Android only lets an app execute files from its native library folder), for
# drawing pictures on the phone itself. Unlike llama-server it is one static program: its ggml is a
# different version from llama.cpp's, so sharing ggml libraries between the two isn't safe.
#
#   NDK_HOME=... scripts/build-sd-android.sh          (SD_TAG overrides the stable-diffusion.cpp release)
set -euo pipefail

TAG=${SD_TAG:-master-929-3f8527a}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$ROOT/src-tauri/target/sd-android
OUT=$ROOT/src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a
NDK=${NDK_HOME:?set NDK_HOME to the Android NDK}

if [ ! -d "$WORK/src-$TAG" ]; then
  git clone --quiet --depth 1 --branch "$TAG" --recurse-submodules --shallow-submodules \
    https://github.com/leejet/stable-diffusion.cpp "$WORK/src-$TAG"
fi

# A static build can't pick its CPU variant at run time, so it targets ARMv8.2 with dot product and
# half-precision maths: every arm64 phone from about 2018 on.
cmake -S "$WORK/src-$TAG" -B "$WORK/build-$TAG" -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28 -DANDROID_STL=c++_static \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
  -DGGML_NATIVE=OFF -DGGML_CPU_ARM_ARCH=armv8.2-a+dotprod+fp16 -DGGML_OPENMP=OFF \
  -DSD_BUILD_EXAMPLES=ON -DSD_SERVER_BUILD_FRONTEND=OFF -DSD_WEBM=OFF
cmake --build "$WORK/build-$TAG" --target sd-server

mkdir -p "$OUT"
cp "$WORK/build-$TAG/bin/sd-server" "$OUT/libsd-server.so"
"$(find "$NDK/toolchains/llvm/prebuilt" -name 'llvm-strip*' | head -1)" --strip-unneeded "$OUT/libsd-server.so"
ls -la "$OUT/libsd-server.so"
