# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

cmake_minimum_required(VERSION 3.16)
project(aviso_ffi_install_smoke CXX)

# Consume the cargo-c install layout the way a real downstream would: discover
# the staged library through its pkg-config package, with no in-tree paths and
# no Rust toolchain. Point PKG_CONFIG_PATH at the staged `lib/pkgconfig` dir.
find_package(PkgConfig REQUIRED)
pkg_check_modules(AVISO_FFI REQUIRED IMPORTED_TARGET aviso_ffi)

add_executable(smoke smoke.cpp)
target_compile_features(smoke PRIVATE cxx_std_17)
target_link_libraries(smoke PRIVATE PkgConfig::AVISO_FFI)
