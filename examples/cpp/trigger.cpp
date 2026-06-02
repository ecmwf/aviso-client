// Watches a stream with a log trigger attached: the trigger appends each
// notification as JSON to a file while the watch runs. The watch stops after a
// fixed count, then the program prints what the trigger wrote. Connection
// settings come from the environment (see aviso_env.hpp).
//
// This only watches. Publish to the stream from another terminal (the publish
// example, or the aviso CLI) to drive it.

#include "aviso_env.hpp"

#include <atomic>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <string>

namespace {

// What to watch, where the trigger logs, and how many to receive before
// stopping.
constexpr const char* kEventType = "test_event";
constexpr const char* kLogPath = "aviso-trigger.log";
constexpr int kStopAfter = 3;

}  // namespace

class StopAfter : public aviso::NotificationHandler {
 public:
  bool on_notification(const aviso::Notification&) override {
    return seen_.fetch_add(1) + 1 < kStopAfter;
  }

 private:
  std::atomic<int> seen_{0};
};

int main() {
  std::remove(kLogPath);

  try {
    aviso::Client client = example::connect();

    aviso::WatchRequest request(kEventType);
    request.add_trigger(aviso::Trigger::log(kLogPath));

    StopAfter handler;
    std::cout << "watching " << kEventType << " with a log trigger writing to "
              << kLogPath << "; stopping after " << kStopAfter
              << " notification(s)\n";
    aviso::Watch watch = client.watch(request, handler);
    watch.wait();

    std::ifstream log(kLogPath);
    std::string line;
    while (std::getline(log, line)) {
      std::cout << "logged: " << line << '\n';
    }
    return 0;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error (" << static_cast<int>(error.error().kind)
              << "): " << error.what() << '\n';
    return 1;
  }
}
