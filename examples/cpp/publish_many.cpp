// Publishes several notifications in one concurrent call with notify_many, then
// prints the per-item results. notify_many sends the whole batch at once and
// returns a JSON array with one entry per input; a per-item failure shows up as
// an "error" entry rather than throwing, so one bad notification does not sink
// the rest. Connection settings come from the environment (see aviso_env.hpp).

#include "aviso_env.hpp"

#include <iostream>
#include <string>

int main() {
  // A JSON array of {event_type, identifier?, payload?} objects. Build it with
  // whatever JSON you like; the facade passes the string straight through.
  const std::string notifications = R"([
    {"event_type": "test_event", "identifier": {"date": "20260101", "time": "0000"}},
    {"event_type": "test_event", "identifier": {"date": "20260101", "time": "0001"}},
    {"event_type": "test_event", "identifier": {"date": "20260101", "time": "0002"}}
  ])";

  try {
    aviso::Client client = example::connect();
    std::cout << "publishing 3 notifications with notify_many\n";

    const std::string results = client.notify_many(notifications);
    std::cout << results << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error (" << static_cast<int>(error.error().kind)
              << "): " << error.what() << '\n';
    return 1;
  }
}
