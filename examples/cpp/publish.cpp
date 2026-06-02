// Publishes one notification, then reads back the stream's schema. Connection
// settings come from the environment (see aviso_env.hpp); against an
// auth-required stream set AVISO_USERNAME and AVISO_PASSWORD to a producer
// account.

#include "aviso_env.hpp"

#include <iostream>
#include <map>
#include <string>

int main() {
  // What to publish: the stream and an identifier matching its schema.
  const std::string event_type = "test_event";
  const std::map<std::string, std::string> identifier = {
      {"date", "20260101"}, {"time", "0000"}};

  try {
    aviso::Client client = example::connect();
    std::cout << "publishing to " << event_type << '\n';

    const std::string response = client.notify(event_type, identifier);
    std::cout << response << '\n';

    const std::string schema = client.schema_for(event_type);
    std::cout << "schema: " << schema << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error (" << static_cast<int>(error.error().kind)
              << "): " << error.what() << '\n';
    return 1;
  }
}
