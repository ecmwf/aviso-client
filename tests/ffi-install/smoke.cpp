// Consumer smoke for the cargo-c install layout: include the facade through
// the installed `aviso_ffi` pkg-config package and call a no-server verb. It
// proves a clean consumer compiles, links, and runs against the staged
// `.a/.so/.dylib` + `.pc` + headers, with no Rust toolchain and no in-tree paths.

#include "aviso.hpp"

#include <iostream>
#include <string>

int main() {
  std::cout << "aviso version: " << aviso::version() << '\n';

  aviso::Client client = aviso::ClientBuilder("http://127.0.0.1:1").build();
  const std::string malformed_identifier = R"(["not-an-object"])";

  bool blocking_rejected = false;
  try {
    static_cast<void>(
        client.notify_json("observations", malformed_identifier));
  } catch (const aviso::Error&) {
    blocking_rejected = true;
  }

  bool async_rejected = false;
  try {
    client.notify_json_async("observations", malformed_identifier).get();
  } catch (const aviso::Error&) {
    async_rejected = true;
  }

  if (!blocking_rejected || !async_rejected) {
    std::cerr << "JSON-valued notify methods accepted a non-object identifier\n";
    return 1;
  }
  return 0;
}
