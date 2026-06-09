// Consumer smoke for the cargo-c install layout: include the facade through
// the installed `aviso_ffi` pkg-config package and call a no-server verb. It
// proves a clean consumer compiles, links, and runs against the staged
// `.a/.so/.dylib` + `.pc` + headers, with no Rust toolchain and no in-tree paths.

#include "aviso.hpp"

#include <iostream>

int main() {
  std::cout << "aviso version: " << aviso::version() << '\n';
  return 0;
}
