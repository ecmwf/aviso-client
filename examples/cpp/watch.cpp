// Watches a stream and prints notifications as they arrive, demonstrating the
// callback watch surface and the one-shared-client pattern: the watch runs on a
// runtime thread while the main program publishes from another thread on the
// same client.
//
// Usage:
//   watch <base_url> [username] [password] [event_type] [count]
//
// Against the e2e stack (see tests/e2e), watch and publish to test_event as the
// producer account, stopping after 3 notifications:
//   ./build/cpp/watch http://localhost:8000 producer-user producer-pass test_event 3
//
// Exit status: 0 if `count` notifications were received; 1 on error or short
// count; 2 on missing arguments.

#include "aviso.hpp"

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <iostream>
#include <map>
#include <string>
#include <thread>

// Counts notifications and asks the watch to stop once it has seen enough.
class CountingHandler : public aviso::NotificationHandler {
 public:
  explicit CountingHandler(int target) : target_(target) {}

  bool on_notification(const aviso::Notification& notification) override {
    ++count_;
    std::cout << "notification " << count_
              << ": event_type=" << notification.event_type()
              << " sequence=" << notification.sequence()
              << " identifier=" << notification.identifier_json()
              << " payload=" << notification.payload_json() << '\n';
    return count_ < target_;
  }

  void on_end(const std::optional<aviso::ErrorInfo>& error) override {
    if (error) {
      std::cerr << "watch ended with error: " << error->message << '\n';
    } else {
      std::cout << "watch ended after " << count_ << " notification(s)\n";
    }
  }

  [[nodiscard]] int count() const { return count_; }

 private:
  int target_;
  int count_ = 0;
};

int main(int argc, char** argv) {
  if (argc < 2) {
    std::cerr << "usage: " << argv[0]
              << " <base_url> [username] [password] [event_type] [count]\n";
    return 2;
  }
  const std::string base_url = argv[1];
  const std::string event_type = argc > 4 ? argv[4] : "test_event";
  const int count = argc > 5 ? std::max(1, std::atoi(argv[5])) : 3;

  try {
    aviso::ClientBuilder builder(base_url);
    if (argc > 3) {
      builder.basic_auth(argv[2], argv[3]);
    }
    aviso::Client client = builder.build();

    CountingHandler handler(count);
    aviso::WatchRequest request(event_type);
    aviso::Watch watch = client.watch(request, handler);

    // Publish from another thread on the same client so the live watch above
    // receives them. This is the one-shared-client pattern.
    std::thread publisher([&] {
      std::this_thread::sleep_for(std::chrono::milliseconds(500));
      for (int i = 0; i < count; ++i) {
        char time[16];
        std::snprintf(time, sizeof(time), "%04d", i);
        const std::map<std::string, std::string> identifier = {
            {"date", "20260101"}, {"time", time}};
        try {
          static_cast<void>(client.notify(event_type, identifier));
        } catch (const aviso::Error& error) {
          std::cerr << "publish failed: " << error.what() << '\n';
          return;
        }
      }
    });

    watch.wait();
    publisher.join();
    return handler.count() >= count ? 0 : 1;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error: kind=" << static_cast<int>(error.error().kind)
              << " message=" << error.what() << '\n';
    return 1;
  }
}
