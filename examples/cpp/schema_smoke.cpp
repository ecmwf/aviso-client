// Minimal end-to-end check of the C++ facade: print the version, build a
// client, and call a blocking verb. With no argument it targets an unreachable
// URL, so it exercises construction and the throwing-error path without a
// server; pass a reachable base URL as the first argument to fetch a real
// schema.
//
// Exit status, so CI catches a broken binding instead of passing blindly:
//   - explicit base URL given: success exits 0, any error exits 1.
//   - no argument (unreachable URL): the only expected failure is a transport
//     error, which exits 0; any other error kind exits 1.

#include "aviso.hpp"

#include <iostream>
#include <string>

int main(int argc, char** argv) {
  const bool explicit_url = argc > 1;
  const std::string base_url = explicit_url ? argv[1] : "http://127.0.0.1:1/";

  std::cout << "aviso version: " << aviso::version() << '\n';

  try {
    aviso::Client client = aviso::ClientBuilder(base_url).build();
    const std::string schema = client.schema();
    std::cout << "schema: " << schema << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    const aviso::ErrorInfo& info = error.error();
    std::cout << "aviso error: kind=" << static_cast<int>(info.kind)
              << " message=" << error.what() << '\n';
    if (explicit_url) {
      return 1;
    }
    return info.kind == AvisoErrorKind_Transport ? 0 : 1;
  }
}
