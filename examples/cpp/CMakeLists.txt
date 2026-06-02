cmake_minimum_required(VERSION 3.16)
project(aviso_cpp_examples CXX)

# A consumer links the prebuilt aviso C ABI library and compiles the
# header-only C++ facade with its own toolchain: no Rust toolchain is needed
# here. Point these at a prebuilt drop, or (for local development) at the
# crate's `include/` and the cargo `target/<profile>` directory.
set(AVISO_FFI_INCLUDE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../crates/aviso-ffi/include"
    CACHE PATH "Directory containing aviso.h and aviso.hpp")
set(AVISO_FFI_LIB_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/debug"
    CACHE PATH "Directory containing libaviso_ffi")

find_library(AVISO_FFI_LIBRARY
    NAMES aviso_ffi
    PATHS "${AVISO_FFI_LIB_DIR}"
    NO_DEFAULT_PATH)
if(NOT AVISO_FFI_LIBRARY)
  message(FATAL_ERROR
      "libaviso_ffi not found in ${AVISO_FFI_LIB_DIR}. "
      "Build it first: cargo build -p aviso-ffi")
endif()

add_executable(schema_smoke schema_smoke.cpp)
target_compile_features(schema_smoke PRIVATE cxx_std_17)
target_include_directories(schema_smoke PRIVATE "${AVISO_FFI_INCLUDE_DIR}")
target_link_libraries(schema_smoke PRIVATE "${AVISO_FFI_LIBRARY}")
set_target_properties(schema_smoke PROPERTIES
    BUILD_RPATH "${AVISO_FFI_LIB_DIR}")

if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(schema_smoke PRIVATE
      -Wall -Wextra -Wpedantic -Wconversion -Wshadow -Wnon-virtual-dtor -Werror)
endif()
