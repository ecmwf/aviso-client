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

find_package(Threads REQUIRED)

function(add_aviso_example name)
  add_executable(${name} ${name}.cpp)
  target_compile_features(${name} PRIVATE cxx_std_17)
  target_include_directories(${name} PRIVATE "${AVISO_FFI_INCLUDE_DIR}")
  target_link_libraries(${name} PRIVATE "${AVISO_FFI_LIBRARY}" Threads::Threads)
  set_target_properties(${name} PROPERTIES BUILD_RPATH "${AVISO_FFI_LIB_DIR}")
  if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(${name} PRIVATE
        -Wall -Wextra -Wpedantic -Wconversion -Wshadow -Wnon-virtual-dtor -Werror)
  endif()
endfunction()

add_aviso_example(schema_smoke)
add_aviso_example(publish)
add_aviso_example(watch)
add_aviso_example(trigger)
add_aviso_example(async)
