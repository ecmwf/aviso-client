#pragma once

// Header-only C++ facade over the stable C ABI in `aviso.h`. It adds RAII
// handles, std::string and std::optional ergonomics, and a throwing
// `aviso::Error`. Compile it with your own C++17 (or later) toolchain over the
// prebuilt library; nothing here needs a Rust toolchain. The C header is the
// generated source of truth; this facade is hand-written and never generated.

#include "aviso.h"

#include <cstdint>
#include <cstdio>
#include <exception>
#include <future>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>

namespace aviso {

// An owned copy of a structured error, safe to keep after the originating
// outcome has been freed.
struct ErrorInfo {
  AvisoErrorKind kind = AvisoErrorKind_Unknown;
  std::uint16_t http_status = 0;
  std::string message;
  std::optional<std::string> request_id;
  std::optional<std::string> trigger_kind;
  std::optional<std::string> error_kind;
};

// Thrown by the facade when a call fails. `what()` is the human-readable
// message; `error()` exposes the full structured detail.
class Error : public std::exception {
 public:
  explicit Error(ErrorInfo info) : info_(std::move(info)) {}

  [[nodiscard]] const char* what() const noexcept override {
    return info_.message.c_str();
  }

  [[nodiscard]] const ErrorInfo& error() const noexcept { return info_; }

 private:
  ErrorInfo info_;
};

namespace detail {

inline std::optional<std::string> optional_string(const char* value) {
  if (value == nullptr) {
    return std::nullopt;
  }
  return std::string(value);
}

// Appends `value` to `out` as a JSON string literal, escaping the characters
// JSON requires. Bytes >= 0x20 (including UTF-8 continuation bytes) pass
// through unchanged; the C ABI only needs valid UTF-8, which the std::string
// already is on a conforming caller.
inline void append_json_string(std::string& out, const std::string& value) {
  out.push_back('"');
  for (const char ch : value) {
    switch (ch) {
      case '"':
        out += "\\\"";
        break;
      case '\\':
        out += "\\\\";
        break;
      case '\b':
        out += "\\b";
        break;
      case '\f':
        out += "\\f";
        break;
      case '\n':
        out += "\\n";
        break;
      case '\r':
        out += "\\r";
        break;
      case '\t':
        out += "\\t";
        break;
      default:
        if (static_cast<unsigned char>(ch) < 0x20) {
          char buffer[7];
          std::snprintf(buffer, sizeof(buffer), "\\u%04x",
                        static_cast<unsigned int>(static_cast<unsigned char>(ch)));
          out += buffer;
        } else {
          out.push_back(ch);
        }
    }
  }
  out.push_back('"');
}

// Serialises a string-to-string map as a compact JSON object, the wire form
// the C ABI's `identifier_json` argument expects.
inline std::string to_identifier_json(
    const std::map<std::string, std::string>& identifier) {
  std::string out = "{";
  bool first = true;
  for (const auto& entry : identifier) {
    if (!first) {
      out.push_back(',');
    }
    first = false;
    append_json_string(out, entry.first);
    out.push_back(':');
    append_json_string(out, entry.second);
  }
  out.push_back('}');
  return out;
}

[[noreturn]] inline void throw_internal(std::string message) {
  ErrorInfo info;
  info.kind = AvisoErrorKind_Internal;
  info.message = std::move(message);
  throw Error(std::move(info));
}

inline ErrorInfo to_error_info(const AvisoError* error) {
  ErrorInfo info;
  if (error == nullptr) {
    info.kind = AvisoErrorKind_Internal;
    info.message = "aviso: outcome reported a failure but carried no error";
    return info;
  }
  info.kind = error->kind;
  info.http_status = error->http_status;
  info.message =
      error->message != nullptr ? std::string(error->message) : std::string();
  info.request_id = optional_string(error->request_id);
  info.trigger_kind = optional_string(error->trigger_kind);
  info.error_kind = optional_string(error->error_kind);
  return info;
}

struct OutcomeDeleter {
  void operator()(AvisoOutcome* outcome) const noexcept {
    aviso_outcome_free(outcome);
  }
};
using OutcomePtr = std::unique_ptr<AvisoOutcome, OutcomeDeleter>;

struct StringDeleter {
  void operator()(char* text) const noexcept { aviso_string_free(text); }
};
using StringPtr = std::unique_ptr<char, StringDeleter>;

struct ClientDeleter {
  void operator()(AvisoClient* client) const noexcept {
    aviso_client_free(client);
  }
};
using ClientPtr = std::unique_ptr<AvisoClient, ClientDeleter>;

struct BuilderDeleter {
  void operator()(AvisoClientBuilder* builder) const noexcept {
    aviso_client_builder_free(builder);
  }
};
using BuilderPtr = std::unique_ptr<AvisoClientBuilder, BuilderDeleter>;

// Throws `aviso::Error` when the outcome carries an error; returns it
// unchanged otherwise so the caller can take a success value out of it.
inline OutcomePtr check(OutcomePtr outcome) {
  if (!outcome) {
    throw_internal("aviso: received a null outcome");
  }
  if (!aviso_outcome_is_ok(outcome.get())) {
    throw Error(to_error_info(aviso_outcome_error(outcome.get())));
  }
  return outcome;
}

// Completion trampolines for the async verbs. Each owns the heap-allocated
// promise passed as `ctx`, fulfils it from the outcome (value or an
// `aviso::Error`), and frees the outcome. A C++ exception is never allowed to
// unwind across the boundary into Rust.
extern "C" inline void async_complete_string(void* ctx, AvisoOutcome* outcome) {
  std::unique_ptr<std::promise<std::string>> promise(
      static_cast<std::promise<std::string>*>(ctx));
  OutcomePtr owned(outcome);
  try {
    if (!owned || !aviso_outcome_is_ok(owned.get())) {
      const AvisoError* error = owned ? aviso_outcome_error(owned.get()) : nullptr;
      promise->set_exception(std::make_exception_ptr(Error(to_error_info(error))));
    } else {
      StringPtr text(aviso_outcome_take_string(owned.get()));
      promise->set_value(text ? std::string(text.get()) : std::string());
    }
  } catch (...) {
  }
}

extern "C" inline void async_complete_void(void* ctx, AvisoOutcome* outcome) {
  std::unique_ptr<std::promise<void>> promise(
      static_cast<std::promise<void>*>(ctx));
  OutcomePtr owned(outcome);
  try {
    if (!owned || !aviso_outcome_is_ok(owned.get())) {
      const AvisoError* error = owned ? aviso_outcome_error(owned.get()) : nullptr;
      promise->set_exception(std::make_exception_ptr(Error(to_error_info(error))));
    } else {
      promise->set_value();
    }
  } catch (...) {
  }
}

}  // namespace detail

// Version of the underlying library.
[[nodiscard]] inline std::string version() {
  return std::string(aviso_version());
}

// A non-owning view of one notification, valid only for the duration of the
// `NotificationHandler::on_notification` call it is passed to. The accessors
// copy out of the borrowed C strings, so the returned values outlive the view.
class Notification {
 public:
  explicit Notification(const AvisoNotification* handle) : handle_(handle) {}

  [[nodiscard]] std::string event_type() const {
    return copy(aviso_notification_event_type(handle_));
  }
  [[nodiscard]] std::uint64_t sequence() const {
    return aviso_notification_sequence(handle_);
  }
  [[nodiscard]] std::string identifier_json() const {
    return copy(aviso_notification_identifier_json(handle_));
  }
  [[nodiscard]] std::string payload_json() const {
    return copy(aviso_notification_payload_json(handle_));
  }

 private:
  static std::string copy(const char* value) {
    return value != nullptr ? std::string(value) : std::string();
  }
  const AvisoNotification* handle_;
};

// A receiver for watch notifications, subclassed by the caller. Both callbacks
// run on a watch (runtime) thread, so they must be thread-safe and must not
// make blocking aviso calls; an exception thrown out of either is caught at the
// boundary (and `on_notification` then requests a graceful stop).
class NotificationHandler {
 public:
  virtual ~NotificationHandler() = default;

  // Called once per notification. Return `false` to request a graceful stop.
  virtual bool on_notification(const Notification& notification) = 0;

  // Called once when the watch ends; `error` is set when it failed. The default
  // does nothing.
  virtual void on_end(const std::optional<ErrorInfo>& error) { (void)error; }
};

class ClientBuilder;
class WatchRequest;
class Watch;

// An RAII client. Move-only; the underlying handle is freed on destruction.
class Client {
 public:
  // Fetches the schema catalog as a compact-JSON string, or throws
  // `aviso::Error`.
  [[nodiscard]] std::string schema() {
    return take_string(aviso_client_schema(handle_.get()), "schema");
  }

  // Fetches the schema for one event type as a compact-JSON string, or throws
  // `aviso::Error`.
  [[nodiscard]] std::string schema_for(const std::string& event_type) {
    return take_string(aviso_client_schema_for(handle_.get(), event_type.c_str()),
                       "schema_for");
  }

  // Publishes a notification and returns the server's response as a
  // compact-JSON string, or throws `aviso::Error`. `identifier` is the
  // string-to-string identifier map; `payload`, when set, is a JSON string.
  [[nodiscard]] std::string notify(
      const std::string& event_type,
      const std::map<std::string, std::string>& identifier = {},
      const std::optional<std::string>& payload = std::nullopt) {
    std::string identifier_json;
    const char* identifier_ptr = nullptr;
    if (!identifier.empty()) {
      identifier_json = detail::to_identifier_json(identifier);
      identifier_ptr = identifier_json.c_str();
    }
    const char* payload_ptr = payload ? payload->c_str() : nullptr;
    return take_string(aviso_client_notify(handle_.get(), event_type.c_str(),
                                           identifier_ptr, payload_ptr),
                       "notify");
  }

  // Wipes every notification for one stream (operator-only), or throws
  // `aviso::Error`.
  void wipe_stream(const std::string& stream_name) {
    detail::check(
        detail::OutcomePtr(aviso_client_wipe_stream(handle_.get(), stream_name.c_str())));
  }

  // Wipes every stream (operator-only), or throws `aviso::Error`.
  void wipe_all() {
    detail::check(detail::OutcomePtr(aviso_client_wipe_all(handle_.get())));
  }

  // Deletes a single notification by its `<event_type>@<sequence>` id
  // (operator-only), or throws `aviso::Error`.
  void delete_notification(const std::string& notification_id) {
    detail::check(detail::OutcomePtr(
        aviso_client_delete_notification(handle_.get(), notification_id.c_str())));
  }

  // Async forms of the blocking verbs. Each returns a std::future that becomes
  // ready when the call completes, holding the response (or throwing the
  // aviso::Error via the future). Unlike the blocking verbs these are safe to
  // call from a watch or async callback.
  [[nodiscard]] std::future<std::string> notify_async(
      const std::string& event_type,
      const std::map<std::string, std::string>& identifier = {},
      const std::optional<std::string>& payload = std::nullopt) {
    std::string identifier_json;
    const char* identifier_ptr = nullptr;
    if (!identifier.empty()) {
      identifier_json = detail::to_identifier_json(identifier);
      identifier_ptr = identifier_json.c_str();
    }
    const char* payload_ptr = payload ? payload->c_str() : nullptr;
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_notify_async(handle_.get(), event_type.c_str(), identifier_ptr,
                              payload_ptr, detail::async_complete_string,
                              promise.release());
    return future;
  }

  [[nodiscard]] std::future<std::string> schema_async() {
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_schema_async(handle_.get(), detail::async_complete_string,
                              promise.release());
    return future;
  }

  [[nodiscard]] std::future<std::string> schema_for_async(
      const std::string& event_type) {
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_schema_for_async(handle_.get(), event_type.c_str(),
                                  detail::async_complete_string,
                                  promise.release());
    return future;
  }

  [[nodiscard]] std::future<void> wipe_stream_async(
      const std::string& stream_name) {
    auto promise = std::make_unique<std::promise<void>>();
    std::future<void> future = promise->get_future();
    aviso_client_wipe_stream_async(handle_.get(), stream_name.c_str(),
                                   detail::async_complete_void,
                                   promise.release());
    return future;
  }

  [[nodiscard]] std::future<void> wipe_all_async() {
    auto promise = std::make_unique<std::promise<void>>();
    std::future<void> future = promise->get_future();
    aviso_client_wipe_all_async(handle_.get(), detail::async_complete_void,
                                promise.release());
    return future;
  }

  [[nodiscard]] std::future<void> delete_notification_async(
      const std::string& notification_id) {
    auto promise = std::make_unique<std::promise<void>>();
    std::future<void> future = promise->get_future();
    aviso_client_delete_notification_async(handle_.get(),
                                           notification_id.c_str(),
                                           detail::async_complete_void,
                                           promise.release());
    return future;
  }

  // Starts a watch, consuming `request`. Notifications are delivered to
  // `handler` on a runtime thread; the returned RAII `Watch` stops and waits in
  // its destructor. `handler` must outlive the returned `Watch`.
  [[nodiscard]] Watch watch(WatchRequest& request, NotificationHandler& handler);

 private:
  friend class ClientBuilder;
  explicit Client(AvisoClient* handle) : handle_(handle) {}

  // Checks the outcome, takes its string success value, and returns it, or
  // throws `aviso::Error`. `what` names the verb for the diagnostic raised when
  // a success outcome unexpectedly carries no string.
  static std::string take_string(AvisoOutcome* raw, const char* what) {
    detail::OutcomePtr outcome = detail::check(detail::OutcomePtr(raw));
    detail::StringPtr text(aviso_outcome_take_string(outcome.get()));
    if (!text) {
      detail::throw_internal(std::string("aviso: ") + what +
                             " succeeded but returned no value");
    }
    return std::string(text.get());
  }

  detail::ClientPtr handle_;
};

// Fluent builder for a `Client`.
class ClientBuilder {
 public:
  explicit ClientBuilder(const std::string& base_url)
      : handle_(aviso_client_builder_new(base_url.c_str())) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a client builder");
    }
  }

  ClientBuilder& basic_auth(const std::string& username,
                            const std::string& password) {
    aviso_client_builder_basic_auth(handle_.get(), username.c_str(),
                                    password.c_str());
    return *this;
  }

  // Builds the client, consuming this builder's handle, or throws
  // `aviso::Error`.
  [[nodiscard]] Client build() {
    AvisoClientBuilder* raw = handle_.release();
    detail::OutcomePtr outcome =
        detail::check(detail::OutcomePtr(aviso_client_builder_build(&raw)));
    AvisoClient* client = aviso_outcome_take_client(outcome.get());
    if (client == nullptr) {
      detail::throw_internal("aviso: build succeeded but returned no client");
    }
    return Client(client);
  }

 private:
  detail::BuilderPtr handle_;
};

namespace detail {

struct WatchRequestDeleter {
  void operator()(AvisoWatchRequest* request) const noexcept {
    aviso_watch_request_free(request);
  }
};
using WatchRequestPtr = std::unique_ptr<AvisoWatchRequest, WatchRequestDeleter>;

struct TriggerDeleter {
  void operator()(AvisoTrigger* trigger) const noexcept {
    aviso_trigger_free(trigger);
  }
};
using TriggerPtr = std::unique_ptr<AvisoTrigger, TriggerDeleter>;

struct WatchDeleter {
  void operator()(AvisoWatch* watch) const noexcept { aviso_watch_free(watch); }
};
using WatchPtr = std::unique_ptr<AvisoWatch, WatchDeleter>;

// Shared between a Watch and the C callbacks via the watch's `ctx`. It outlives
// the watch task (the Watch keeps it alive until after stop+wait), so the
// callbacks can dereference it safely.
struct WatchState {
  NotificationHandler* handler = nullptr;
};

// C-ABI trampolines. They translate the C callbacks into virtual calls and
// stop any C++ exception from unwinding across the boundary into Rust.
extern "C" inline bool watch_on_notification(void* ctx,
                                             const AvisoNotification* notification) {
  auto* state = static_cast<WatchState*>(ctx);
  try {
    return state->handler->on_notification(Notification(notification));
  } catch (...) {
    return false;
  }
}

extern "C" inline void watch_on_end(void* ctx, AvisoOutcome* outcome) {
  auto* state = static_cast<WatchState*>(ctx);
  OutcomePtr owned(outcome);
  std::optional<ErrorInfo> error;
  if (owned && !aviso_outcome_is_ok(owned.get())) {
    error = to_error_info(aviso_outcome_error(owned.get()));
  }
  try {
    state->handler->on_end(error);
  } catch (...) {
  }
}

}  // namespace detail

// HTTP method for the webhook trigger.
enum class HttpMethod {
  Post = AvisoHttpMethod_Post,
  Get = AvisoHttpMethod_Get,
  Put = AvisoHttpMethod_Put,
  Patch = AvisoHttpMethod_Patch,
  Delete = AvisoHttpMethod_Delete,
};

// A per-notification side effect, built by a static factory and tuned with the
// chainable setters, then attached to a WatchRequest with `add_trigger`. A
// setter that does not apply to the trigger's kind is ignored. A bad argument
// is reported through the watch's on_end when it starts.
class Trigger {
 public:
  static Trigger echo() { return Trigger(aviso_trigger_echo()); }
  static Trigger log(const std::string& path) {
    return Trigger(aviso_trigger_log(path.c_str()));
  }
  static Trigger command(const std::string& cmd) {
    return Trigger(aviso_trigger_command(cmd.c_str()));
  }
  static Trigger webhook(const std::string& url) {
    return Trigger(aviso_trigger_webhook(url.c_str()));
  }
  static Trigger teams(const std::string& url) {
    return Trigger(aviso_trigger_teams(url.c_str()));
  }
  static Trigger post(const std::string& url) {
    return Trigger(aviso_trigger_post(url.c_str()));
  }

  Trigger& label(const std::string& name) {
    aviso_trigger_set_label(handle_.get(), name.c_str());
    return *this;
  }
  Trigger& retries(std::uint32_t retries) {
    aviso_trigger_set_retries(handle_.get(), retries);
    return *this;
  }
  Trigger& required(bool required) {
    aviso_trigger_set_required(handle_.get(), required);
    return *this;
  }
  Trigger& timeout_secs(std::uint64_t seconds) {
    aviso_trigger_set_timeout_secs(handle_.get(), seconds);
    return *this;
  }
  Trigger& fail_fast(bool on) {
    aviso_trigger_set_fail_fast(handle_.get(), on);
    return *this;
  }
  Trigger& method(HttpMethod method) {
    aviso_trigger_set_method(handle_.get(),
                             static_cast<std::uint32_t>(method));
    return *this;
  }
  Trigger& header(const std::string& name, const std::string& value) {
    aviso_trigger_set_header(handle_.get(), name.c_str(), value.c_str());
    return *this;
  }
  Trigger& body_template(const std::string& body) {
    aviso_trigger_set_body_template(handle_.get(), body.c_str());
    return *this;
  }
  Trigger& env(const std::string& key, const std::string& value) {
    aviso_trigger_set_env(handle_.get(), key.c_str(), value.c_str());
    return *this;
  }
  Trigger& working_dir(const std::string& dir) {
    aviso_trigger_set_working_dir(handle_.get(), dir.c_str());
    return *this;
  }

 private:
  friend class WatchRequest;
  explicit Trigger(AvisoTrigger* handle) : handle_(handle) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a trigger");
    }
  }
  AvisoTrigger* release() { return handle_.release(); }
  detail::TriggerPtr handle_;
};

// Fluent builder for a watch request. Defaults to a live watch of `event_type`;
// the `*_from_*` setters add a resume position or switch to replay-only.
class WatchRequest {
 public:
  explicit WatchRequest(const std::string& event_type)
      : handle_(aviso_watch_request_new(event_type.c_str())) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a watch request");
    }
  }

  WatchRequest& filter_json(const std::string& json) {
    aviso_watch_request_set_filter_json(handle_.get(), json.c_str());
    return *this;
  }
  WatchRequest& watch_from_sequence(std::uint64_t sequence) {
    aviso_watch_request_watch_from_sequence(handle_.get(), sequence);
    return *this;
  }
  WatchRequest& watch_from_date(const std::string& date) {
    aviso_watch_request_watch_from_date(handle_.get(), date.c_str());
    return *this;
  }
  WatchRequest& replay_from_sequence(std::uint64_t sequence) {
    aviso_watch_request_replay_from_sequence(handle_.get(), sequence);
    return *this;
  }
  WatchRequest& replay_from_date(const std::string& date) {
    aviso_watch_request_replay_from_date(handle_.get(), date.c_str());
    return *this;
  }

  // Attaches a trigger, consuming it.
  WatchRequest& add_trigger(Trigger trigger) {
    AvisoTrigger* raw = trigger.release();
    aviso_watch_request_add_trigger(handle_.get(), &raw);
    return *this;
  }

 private:
  friend class Client;
  AvisoWatchRequest* release() { return handle_.release(); }
  detail::WatchRequestPtr handle_;
};

// An RAII watch. Move-only. The destructor stops the watch and waits for it to
// finish (so the handler's `on_end` has returned) before releasing the handle.
class Watch {
 public:
  Watch(Watch&&) = default;
  Watch& operator=(Watch&&) = default;
  Watch(const Watch&) = delete;
  Watch& operator=(const Watch&) = delete;

  ~Watch() {
    if (handle_) {
      aviso_watch_stop(handle_.get());
      detail::OutcomePtr drained(aviso_watch_wait(handle_.get()));
      // Only an InvalidUsage outcome means wait was refused (destroyed on a
      // runtime/callback thread) and the task may still call on_end with `ctx`;
      // leak the state then rather than free it out from under a live task. Any
      // other outcome (ok, or a task panic) means the task has ended, so the
      // normal path frees the state.
      if (drained != nullptr && !aviso_outcome_is_ok(drained.get())) {
        const AvisoError* error = aviso_outcome_error(drained.get());
        if (error != nullptr && error->kind == AvisoErrorKind_InvalidUsage) {
          static_cast<void>(state_.release());
        }
      }
    }
  }

  // Requests a graceful stop. Nonblocking and idempotent.
  void stop() { aviso_watch_stop(handle_.get()); }

  // Blocks until the watch has ended and the handler's `on_end` has returned.
  // Throws `aviso::Error` if called from inside a callback.
  void wait() { detail::check(detail::OutcomePtr(aviso_watch_wait(handle_.get()))); }

 private:
  friend class Client;
  Watch(AvisoWatch* handle, std::unique_ptr<detail::WatchState> state)
      : handle_(handle), state_(std::move(state)) {}

  detail::WatchPtr handle_;
  std::unique_ptr<detail::WatchState> state_;
};

inline Watch Client::watch(WatchRequest& request, NotificationHandler& handler) {
  auto state = std::make_unique<detail::WatchState>();
  state->handler = &handler;
  AvisoWatchRequest* raw_request = request.release();
  AvisoWatch* watch = aviso_client_watch(handle_.get(), &raw_request,
                                         detail::watch_on_notification,
                                         detail::watch_on_end, state.get());
  if (watch == nullptr) {
    detail::throw_internal("aviso: failed to start the watch");
  }
  return Watch(watch, std::move(state));
}

}  // namespace aviso
