// Fires async verbs and waits on their futures. The async forms return a
// std::future immediately and run on a background thread, so several calls can
// be in flight at once; .get() blocks for the result and rethrows any
// aviso::Error. Connection settings come from the environment (see
// aviso_env.hpp).

#include "aviso_env.hpp"

#include <future>
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

    // Two calls in flight at once.
    std::future<std::string> published =
        client.notify_async(event_type, identifier);
    std::future<std::string> catalogue = client.schema_async();

    std::cout << "notify: " << published.get() << '\n';
    std::cout << "schema: " << catalogue.get() << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error (" << static_cast<int>(error.error().kind)
              << "): " << error.what() << '\n';
    return 1;
  }
}
