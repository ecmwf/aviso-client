// Minimal end-to-end check of the C++ facade: print the version, build a
// client, and call a blocking verb. Pointed at an unreachable URL by default,
// so it exercises the success path of construction and the throwing-error path
// of a verb without needing a running server. Pass a reachable base URL as the
// first argument to fetch a real schema.

#include "aviso.hpp"

#include <iostream>
#include <string>

int main(int argc, char** argv) {
  const std::string base_url = argc > 1 ? argv[1] : "http://127.0.0.1:1/";

  std::cout << "aviso version: " << aviso::version() << '\n';

  try {
    aviso::Client client = aviso::ClientBuilder(base_url).build();
    const std::string schema = client.schema();
    std::cout << "schema: " << schema << '\n';
  } catch (const aviso::Error& error) {
    const aviso::ErrorInfo& info = error.error();
    std::cout << "aviso error: kind=" << static_cast<int>(info.kind)
              << " message=" << error.what() << '\n';
  }

  return 0;
}
