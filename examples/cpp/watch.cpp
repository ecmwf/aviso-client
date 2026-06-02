// Watches a stream and prints each notification as it arrives, stopping after a
// fixed count. Connection settings come from the environment (see
// aviso_env.hpp).
//
// This only watches. Publish to the stream from another terminal (the publish
// example, or the aviso CLI) to see notifications arrive.

#include "aviso_env.hpp"

#include <atomic>
#include <iostream>
#include <optional>

namespace {

// What to watch: the stream and how many notifications to print before
// stopping.
constexpr const char* kEventType = "test_event";
constexpr int kStopAfter = 3;

}  // namespace

class PrintingHandler : public aviso::NotificationHandler {
 public:
  bool on_notification(const aviso::Notification& notification) override {
    const int seen = seen_.fetch_add(1) + 1;
    std::cout << "#" << notification.sequence() << "  "
              << notification.identifier_json() << "  "
              << notification.payload_json() << '\n';
    return seen < kStopAfter;
  }

  void on_end(const std::optional<aviso::ErrorInfo>& error) override {
    if (error) {
      std::cerr << "watch ended: " << error->message << '\n';
    }
  }

 private:
  std::atomic<int> seen_{0};
};

int main() {
  try {
    aviso::Client client = example::connect();

    aviso::WatchRequest request(kEventType);
    // Narrow the stream to matching identifiers by uncommenting:
    // request.filter_json(R"({"date": "20260101"})");

    PrintingHandler handler;
    std::cout << "watching " << kEventType << "; stopping after " << kStopAfter
              << " notification(s)\n";
    aviso::Watch watch = client.watch(request, handler);
    watch.wait();
    return 0;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error (" << static_cast<int>(error.error().kind)
              << "): " << error.what() << '\n';
    return 1;
  }
}
