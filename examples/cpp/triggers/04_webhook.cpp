// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// POST each notification to an HTTP endpoint.
//
// webhook() sends the notification as JSON to a URL. body_template() lets
// you shape the body with fields from the notification, and header() adds
// whatever the receiver needs. teams() and post() take the same setters, so
// once you know webhook() you know them too.
//
// There is no receiver in the e2e stack, so this example opens a tiny one on
// a loopback port with plain sockets: enough to accept the POST and print
// what arrived. Real code would point the trigger at a real service.
//
// Expect: the receiver prints the body of each POST, then exit.

#include "../common.hpp"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>

#include <atomic>
#include <cctype>
#include <cerrno>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <string>
#include <thread>

namespace {

constexpr int kStopAfter = 3;

// A receiver that accepts kStopAfter POSTs, prints their bodies, and exits.
// It is deliberately minimal: no HTTP library, just enough to be a target.
class Receiver {
 public:
  Receiver() {
    fd_ = ::socket(AF_INET, SOCK_STREAM, 0);
    if (fd_ < 0) {
      fail("socket");
    }
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0;  // let the kernel pick
    socklen_t len = sizeof addr;
    if (::bind(fd_, reinterpret_cast<sockaddr*>(&addr), sizeof addr) < 0 ||
        ::listen(fd_, 8) < 0 ||
        ::getsockname(fd_, reinterpret_cast<sockaddr*>(&addr), &len) < 0) {
      const int saved = errno;
      ::close(fd_);
      errno = saved;
      fail("bind");
    }
    port_ = ntohs(addr.sin_port);
  }
  ~Receiver() { ::close(fd_); }

  // Serves in the background until either count POSTs have arrived or
  // close_door() is called. Joins on destruction, so an exception anywhere
  // in main() cannot leave the thread running or abort the program.
  class Serving {
   public:
    Serving(Receiver& receiver, int count)
        : receiver_(receiver), thread_([&receiver, count] { receiver.serve(count); }) {}
    ~Serving() {
      receiver_.close_door();
      thread_.join();
    }
    Serving(const Serving&) = delete;
    Serving& operator=(const Serving&) = delete;

   private:
    Receiver& receiver_;
    std::thread thread_;
  };

  std::string url() const { return "http://127.0.0.1:" + std::to_string(port_) + "/hook"; }

  // Unblocks accept() so serve() returns even if fewer POSTs came than
  // expected. Called when the watch has ended, whatever the reason.
  void close_door() { ::shutdown(fd_, SHUT_RDWR); }

  void serve(int count) {
    for (int i = 0; i < count; ++i) {
      const int conn = ::accept(fd_, nullptr, nullptr);
      if (conn < 0) {
        return;
      }
      const std::string body = read_body(conn);
      std::cout << "received: " << body << '\n';
      const char* ok = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
      static_cast<void>(::write(conn, ok, std::strlen(ok)));
      ::close(conn);
    }
  }

 private:
  [[noreturn]] static void fail(const char* what) {
    throw std::runtime_error(std::string("receiver: ") + what + ": " + std::strerror(errno));
  }

  // Reads headers, then exactly Content-Length bytes of body. The trigger
  // always sends a Content-Length.
  static std::string read_body(int conn) {
    std::string request;
    char buf[4096];
    std::size_t header_end = std::string::npos;
    std::size_t want = 0;
    for (;;) {
      if (header_end == std::string::npos) {
        header_end = request.find("\r\n\r\n");
        if (header_end != std::string::npos) {
          want = content_length(request.substr(0, header_end));
        }
      }
      if (header_end != std::string::npos && request.size() >= header_end + 4 + want) {
        break;
      }
      const ssize_t n = ::read(conn, buf, sizeof buf);
      if (n <= 0) {
        break;
      }
      request.append(buf, static_cast<std::size_t>(n));
    }
    return header_end == std::string::npos ? "" : request.substr(header_end + 4, want);
  }

  // Header names are case-insensitive, and this client sends them in
  // lowercase, so compare in lowercase.
  static std::size_t content_length(std::string headers) {
    for (char& c : headers) {
      c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    }
    const auto at = headers.find("content-length:");
    if (at == std::string::npos) {
      return 0;
    }
    return static_cast<std::size_t>(std::stoul(headers.substr(at + 15)));
  }

  int fd_ = -1;
  int port_ = 0;
};

class CountOnly : public example::Handler {
 public:
  bool on_notification(const aviso::Notification&) override {
    return seen_.fetch_add(1) + 1 < kStopAfter;
  }

 private:
  std::atomic<int> seen_{0};
};

}  // namespace

int main() {
  return example::run([] {
    aviso::Client client = example::connect();

    Receiver receiver;
    Receiver::Serving serving(receiver, kStopAfter);

    // The body template names notification fields; the receiver sees the
    // rendered result. Headers are plain key/value pairs.
    aviso::Trigger hook = aviso::Trigger::webhook(receiver.url());
    hook.method(aviso::HttpMethod::Post)
        .header("X-Source", "cpp-example")
        .body_template(
            R"({"event": "{{ notification.event_type }}", )"
            R"("seq": {{ notification.sequence }}, )"
            R"("date": "{{ notification.identifier.date }}"})")
        .timeout_secs(5)
        .label("demo-hook");

    aviso::WatchRequest request(example::kEventType);
    request.add_trigger(std::move(hook));

    CountOnly handler;
    aviso::Watch watch = client.watch(request, handler);
    // Triggers run before the handler sees a notification, so by the time
    // the third one has been counted, the third POST has been served. The
    // receiver's thread is joined when `serving` goes out of scope.
    return example::finish(watch, handler);
  });
}
