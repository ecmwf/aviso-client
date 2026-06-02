// Watches a stream with a log trigger attached: the trigger appends each
// notification as JSON to a file while the watch runs. The program publishes a
// few notifications from another thread, stops after receiving them, then
// prints the log file the trigger wrote.
//
// Usage:
//   trigger <base_url> [username] [password] [event_type] [count]
//
// Against the e2e stack (see tests/e2e), as the producer account:
//   ./build/cpp/trigger http://localhost:8000 producer-user producer-pass test_event 3
//
// Exit status: 0 if the log file received `count` lines; 1 on error or short
// count; 2 on missing arguments.

#include "aviso.hpp"

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <map>
#include <string>
#include <thread>

// Stops the watch once it has seen the target number of notifications. The
// trigger does the printing (to the log file); this just bounds the run.
class StopAfter : public aviso::NotificationHandler {
 public:
  explicit StopAfter(int target) : target_(target) {}
  bool on_notification(const aviso::Notification&) override {
    return count_.fetch_add(1) + 1 < target_;
  }

 private:
  int target_;
  std::atomic<int> count_{0};
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
  const std::string log_path = "aviso-trigger.log";
  std::remove(log_path.c_str());

  try {
    aviso::ClientBuilder builder(base_url);
    if (argc > 3) {
      builder.basic_auth(argv[2], argv[3]);
    }
    aviso::Client client = builder.build();

    StopAfter handler(count);
    aviso::WatchRequest request(event_type);
    request.add_trigger(aviso::Trigger::log(log_path));
    aviso::Watch watch = client.watch(request, handler);

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
          watch.stop();
          return;
        }
      }
    });

    watch.wait();
    publisher.join();

    std::ifstream log(log_path);
    std::string line;
    int lines = 0;
    while (std::getline(log, line)) {
      std::cout << "logged: " << line << '\n';
      ++lines;
    }
    return lines >= count ? 0 : 1;
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error: kind=" << static_cast<int>(error.error().kind)
              << " message=" << error.what() << '\n';
    return 1;
  }
}
