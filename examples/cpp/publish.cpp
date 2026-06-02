// Publishes a notification and then fetches the schema for its event type,
// against a running server. Unlike schema_smoke, this needs a reachable server
// and (for an auth-required stream) producer credentials.
//
// Usage:
//   publish <base_url> [username] [password] [event_type]
//
// With the e2e stack (see tests/e2e), publish to the auth-required test_event
// stream as the producer account:
//   ./build/cpp/publish http://localhost:8000 producer-user producer-pass
//
// Exit status: success exits 0; any aviso::Error exits 1; missing arguments
// exit 2.

#include "aviso.hpp"

#include <iostream>
#include <map>
#include <optional>
#include <string>

int main(int argc, char** argv) {
  if (argc < 2) {
    std::cerr << "usage: " << argv[0]
              << " <base_url> [username] [password] [event_type]\n";
    return 2;
  }
  const std::string base_url = argv[1];
  const std::string event_type = argc > 4 ? argv[4] : "test_event";

  std::cout << "aviso version: " << aviso::version() << '\n';

  try {
    aviso::ClientBuilder builder(base_url);
    if (argc > 3) {
      builder.basic_auth(argv[2], argv[3]);
    }
    aviso::Client client = builder.build();

    const std::map<std::string, std::string> identifier = {
        {"date", "20260101"}, {"time", "0000"}};
    const std::string response = client.notify(event_type, identifier);
    std::cout << "notify: " << response << '\n';

    std::cout << "schema_for: " << client.schema_for(event_type) << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    const aviso::ErrorInfo& info = error.error();
    std::cout << "aviso error: kind=" << static_cast<int>(info.kind)
              << " message=" << error.what() << '\n';
    return 1;
  }
}
