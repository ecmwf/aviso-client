/*
 * SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
 * SPDX-License-Identifier: Apache-2.0
 */

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

// Serialises a string-to-string map as a compact JSON object. Use the JSON
// overload when an identifier contains arrays, objects, or other JSON values.
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

[[noreturn]] inline void throw_usage(std::string message) {
  ErrorInfo info;
  info.kind = AvisoErrorKind_InvalidUsage;
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
      if (!text) {
        // A success that carries no string is an internal protocol bug, not an
        // empty value; surface it like the blocking facade does.
        ErrorInfo info;
        info.kind = AvisoErrorKind_Internal;
        info.message = "aviso: async call succeeded but returned no value";
        promise->set_exception(std::make_exception_ptr(Error(std::move(info))));
      } else {
        promise->set_value(std::string(text.get()));
      }
    }
  } catch (...) {
    // Building the value/exception threw (e.g. bad_alloc): still fulfil the
    // promise so .get() raises something deterministic instead of
    // broken_promise. The inner guard covers an already-satisfied promise.
    try {
      promise->set_exception(std::current_exception());
    } catch (...) {
    }
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
    // Fulfil the promise even if the above threw, so .get() raises something
    // deterministic instead of broken_promise.
    try {
      promise->set_exception(std::current_exception());
    } catch (...) {
    }
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
// make blocking aviso calls. An exception thrown out of `on_notification` is
// caught at the boundary and stops the watch; `on_end` then receives an
// `AvisoErrorKind_Internal` error whose message names the exception. An
// exception thrown out of `on_end` is caught and dropped.
class NotificationHandler {
 public:
  virtual ~NotificationHandler() = default;

  // Called once per notification. Return `false` to request a graceful stop.
  virtual bool on_notification(const Notification& notification) = 0;

  // Called once when the watch ends; `error` is set when it failed. The default
  // does nothing.
  virtual void on_end(const std::optional<ErrorInfo>& error) { (void)error; }
};

// A receiver for a merged watch (`Client::watch_many`), subclassed by the
// caller. Each callback receives the name the watch was given in the
// `WatchSet`. The callbacks run on a watch (runtime) thread, one at a time,
// so they must be thread-safe and must not make blocking aviso calls. An
// exception thrown out of `on_notification` or `on_error` is caught at the
// boundary and stops every watch; `on_end` then receives an
// `AvisoErrorKind_Internal` error whose message names the exception. An
// exception thrown out of `on_end` is caught and dropped.
class MultiNotificationHandler {
 public:
  virtual ~MultiNotificationHandler() = default;

  // Called once per notification, with the name of its watch. Return `false`
  // to stop every watch.
  virtual bool on_notification(const std::string& name,
                               const Notification& notification) = 0;

  // Called when the watch `name` fails; the message begins with
  // `watch '<name>': `. Return `true` to drop that watch and keep reading the
  // others, or `false` to stop every watch, in which case `on_end` receives
  // the same error. The default stops.
  virtual bool on_error(const std::string& name, const ErrorInfo& error) {
    (void)name;
    (void)error;
    return false;
  }

  // Called once when the merged watch ends. `error` is empty when every watch
  // ended, including after failures `on_error` chose to continue past, or
  // after a stop. It is set when the set was invalid, when `on_error`
  // returned `false`, or when every watch failed.
  virtual void on_end(const std::optional<ErrorInfo>& error) { (void)error; }
};

class ClientBuilder;
class WatchRequest;
class WatchSet;
class Watch;

// An RAII client. Move-only; the underlying handle is freed on destruction.
// Every method of a moved-from client throws `aviso::Error`
// (`AvisoErrorKind_InvalidUsage`), the async verbs before starting anything.
class Client {
 public:
  // Fetches the schema catalog as a compact-JSON string, or throws
  // `aviso::Error`.
  [[nodiscard]] std::string schema() {
    return take_string(aviso_client_schema(live()), "schema");
  }

  // Fetches the schema for one event type as a compact-JSON string, or throws
  // `aviso::Error`.
  [[nodiscard]] std::string schema_for(const std::string& event_type) {
    return take_string(aviso_client_schema_for(live(), event_type.c_str()),
                       "schema_for");
  }

  // Publishes a notification and returns the server's response as a
  // compact-JSON string, or throws `aviso::Error`. `identifier` is the
  // string-to-string identifier map; `payload`, when set, is a JSON string.
  // Use `notify_json` when identifier values are structured JSON.
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
    return take_string(aviso_client_notify(live(), event_type.c_str(),
                                           identifier_ptr, payload_ptr),
                       "notify");
  }

  // Publishes a notification whose identifier values may have any JSON shape.
  // `identifier_json` must be a JSON object, for example
  // {"point_cloud":[[46,8],[47,9]]}.
  [[nodiscard]] std::string notify_json(
      const std::string& event_type, const std::string& identifier_json,
      const std::optional<std::string>& payload = std::nullopt) {
    const char* payload_ptr = payload ? payload->c_str() : nullptr;
    return take_string(aviso_client_notify(live(), event_type.c_str(),
                                           identifier_json.c_str(), payload_ptr),
                       "notify_json");
  }

  // Publishes many notifications concurrently and returns a compact-JSON array
  // of per-item results, or throws `aviso::Error`. `notifications_json` is a
  // JSON array of {event_type, identifier?, payload?} objects; `max_concurrency`
  // caps in-flight requests (0 selects a default). A per-item failure is
  // reported in the returned array, not thrown; only a malformed array throws.
  [[nodiscard]] std::string notify_many(const std::string& notifications_json,
                                         std::size_t max_concurrency = 0) {
    return take_string(
        aviso_client_notify_many(live(), notifications_json.c_str(),
                                 max_concurrency),
        "notify_many");
  }

  // Wipes every notification for one stream (operator-only), or throws
  // `aviso::Error`.
  void wipe_stream(const std::string& stream_name) {
    detail::check(
        detail::OutcomePtr(aviso_client_wipe_stream(live(), stream_name.c_str())));
  }

  // Wipes every stream (operator-only), or throws `aviso::Error`.
  void wipe_all() {
    detail::check(detail::OutcomePtr(aviso_client_wipe_all(live())));
  }

  // Deletes a single notification by its `<event_type>@<sequence>` id
  // (operator-only), or throws `aviso::Error`.
  void delete_notification(const std::string& notification_id) {
    detail::check(detail::OutcomePtr(
        aviso_client_delete_notification(live(), notification_id.c_str())));
  }

  // Async forms of the blocking verbs. Each returns a std::future that becomes
  // ready when the call completes, holding the response (or throwing the
  // aviso::Error via the future). Unlike the blocking verbs these are safe to
  // call from a watch or async callback.
  [[nodiscard]] std::future<std::string> notify_async(
      const std::string& event_type,
      const std::map<std::string, std::string>& identifier = {},
      const std::optional<std::string>& payload = std::nullopt) {
    AvisoClient* client = live();
    std::string identifier_json;
    const char* identifier_ptr = nullptr;
    if (!identifier.empty()) {
      identifier_json = detail::to_identifier_json(identifier);
      identifier_ptr = identifier_json.c_str();
    }
    const char* payload_ptr = payload ? payload->c_str() : nullptr;
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_notify_async(client, event_type.c_str(), identifier_ptr,
                              payload_ptr, detail::async_complete_string,
                              promise.release());
    return future;
  }

  // Async form of `notify_json` for JSON-valued identifiers.
  [[nodiscard]] std::future<std::string> notify_json_async(
      const std::string& event_type, const std::string& identifier_json,
      const std::optional<std::string>& payload = std::nullopt) {
    AvisoClient* client = live();
    const char* payload_ptr = payload ? payload->c_str() : nullptr;
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_notify_async(client, event_type.c_str(),
                              identifier_json.c_str(), payload_ptr,
                              detail::async_complete_string, promise.release());
    return future;
  }

  [[nodiscard]] std::future<std::string> schema_async() {
    AvisoClient* client = live();
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_schema_async(client, detail::async_complete_string,
                              promise.release());
    return future;
  }

  [[nodiscard]] std::future<std::string> schema_for_async(
      const std::string& event_type) {
    AvisoClient* client = live();
    auto promise = std::make_unique<std::promise<std::string>>();
    std::future<std::string> future = promise->get_future();
    aviso_client_schema_for_async(client, event_type.c_str(),
                                  detail::async_complete_string,
                                  promise.release());
    return future;
  }

  [[nodiscard]] std::future<void> wipe_stream_async(
      const std::string& stream_name) {
    AvisoClient* client = live();
    auto promise = std::make_unique<std::promise<void>>();
    std::future<void> future = promise->get_future();
    aviso_client_wipe_stream_async(client, stream_name.c_str(),
                                   detail::async_complete_void,
                                   promise.release());
    return future;
  }

  [[nodiscard]] std::future<void> wipe_all_async() {
    AvisoClient* client = live();
    auto promise = std::make_unique<std::promise<void>>();
    std::future<void> future = promise->get_future();
    aviso_client_wipe_all_async(client, detail::async_complete_void,
                                promise.release());
    return future;
  }

  [[nodiscard]] std::future<void> delete_notification_async(
      const std::string& notification_id) {
    AvisoClient* client = live();
    auto promise = std::make_unique<std::promise<void>>();
    std::future<void> future = promise->get_future();
    aviso_client_delete_notification_async(client,
                                           notification_id.c_str(),
                                           detail::async_complete_void,
                                           promise.release());
    return future;
  }

  // Starts a watch, consuming `request`. Notifications are delivered to
  // `handler` on a runtime thread; the returned RAII `Watch` stops and waits in
  // its destructor. `handler` must outlive the returned `Watch`. Throws
  // `aviso::Error` (`AvisoErrorKind_InvalidUsage`) if `request` was already
  // used.
  [[nodiscard]] Watch watch(WatchRequest& request, NotificationHandler& handler);

  // Starts one watch per request in `watches`, consuming the set, and
  // delivers their notifications to `handler` with the name of each watch.
  // Watches are read in turn, so a busy watch cannot delay a quiet one. The
  // returned RAII `Watch` stops and waits for every watch in its destructor.
  // `handler` must outlive the returned `Watch`. Throws `aviso::Error`
  // (`AvisoErrorKind_InvalidUsage`) if `watches` was already used.
  [[nodiscard]] Watch watch_many(WatchSet& watches,
                                 MultiNotificationHandler& handler);

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

  // The handle, or throws if this client was moved from. The async verbs
  // call it before creating their promise, so a throw leaves nothing behind.
  [[nodiscard]] AvisoClient* live() const {
    if (!handle_) {
      detail::throw_usage("aviso: this Client was moved from");
    }
    return handle_.get();
  }

  detail::ClientPtr handle_;
};

// Fluent builder for a `Client`.
class ClientBuilder {
 public:
  explicit ClientBuilder(const std::string& base_url)
      : ClientBuilder(aviso_client_builder_new(base_url.c_str())) {}

  // Starts from the aviso config file (`~/.config/aviso/config.yaml`, or
  // `AVISO_CLIENT_CONFIG_FILE`) and a credential found the way the `aviso`
  // binary finds one. A missing file sets nothing. Setters called afterwards
  // replace what the file said. A file that cannot be used, or a found
  // credential that may not travel to the configured address, throws from
  // `build()`.
  static ClientBuilder from_file() {
    return ClientBuilder(aviso_client_builder_from_file());
  }

  // As `from_file()`, reading a specific file. The path must exist.
  static ClientBuilder from_file(const std::string& path) {
    return ClientBuilder(aviso_client_builder_from_file_at(path.c_str()));
  }

  // Starts with nothing named in code, the way a program on a machine set
  // up for the `aviso` command wants to. The address comes from
  // `AVISO_BASE_URL`, then the config file; the credential from
  // `AVISO_TOKEN` or `AVISO_USERNAME` with `AVISO_PASSWORD`, then the file,
  // then the credentials file; timeouts and TLS settings from the file.
  // Setters called afterwards replace what was found. No address anywhere,
  // or a found credential headed for a plain http address that is not
  // loopback, throws from `build()`. `describe()` shows what was chosen.
  static ClientBuilder from_environment() {
    return ClientBuilder(aviso_client_builder_from_environment());
  }

  // The settings a client built from this builder would use and where each
  // came from, one per line, without building. Nothing in it is a secret:
  // the credential appears by kind and source, the address without any
  // `user:password@`. Log it, or paste it into a ticket. Throws
  // `aviso::Error` for an error the builder already holds, or a config
  // file that cannot be read.
  [[nodiscard]] std::string describe() const {
    return Client::take_string(aviso_client_builder_describe(live()),
                               "describe");
  }

  // Sets or replaces the server address. After `from_file()` this overrides
  // the file, or supplies an address the file did not have.
  ClientBuilder& base_url(const std::string& url) {
    aviso_client_builder_base_url(live(), url.c_str());
    return *this;
  }

  ClientBuilder& basic_auth(const std::string& username,
                            const std::string& password) {
    aviso_client_builder_basic_auth(live(), username.c_str(),
                                    password.c_str());
    return *this;
  }

  // Sends `token` as a Bearer credential. An empty token throws from
  // `build()`.
  ClientBuilder& bearer_auth(const std::string& token) {
    aviso_client_builder_bearer_auth(live(), token.c_str());
    return *this;
  }

  // Looks for a credential in the environment, then the config file, then
  // the credentials file, and uses the first one found. Finding nothing
  // changes nothing: a credential set earlier with `bearer_auth` or
  // `basic_auth` stays, and a builder with none stays anonymous. An unusable
  // source throws from `build()`.
  // A credential found this way is not sent to a plain http address unless
  // it is loopback; `build()` throws instead. Name it with `bearer_auth` or
  // `basic_auth` when you hold it and the address is deliberate.
  ClientBuilder& discover_auth() {
    aviso_client_builder_discover_auth(live());
    return *this;
  }

  // Builds the client, consuming this builder's handle, or throws
  // `aviso::Error`. After `build()` the builder is used up: calling any method
  // on it throws `aviso::Error` (`AvisoErrorKind_InvalidUsage`).
  [[nodiscard]] Client build() {
    static_cast<void>(live());
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
  explicit ClientBuilder(AvisoClientBuilder* raw) : handle_(raw) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a client builder");
    }
  }

  // The handle, or throws if this builder was already built or moved from.
  [[nodiscard]] AvisoClientBuilder* live() const {
    if (!handle_) {
      detail::throw_usage(
          "aviso: this ClientBuilder was already used (built or moved from)");
    }
    return handle_.get();
  }

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

struct WatchListDeleter {
  void operator()(AvisoWatchList* list) const noexcept {
    aviso_watch_list_free(list);
  }
};
using WatchListPtr = std::unique_ptr<AvisoWatchList, WatchListDeleter>;

// Shared between a Watch and the C callbacks via the watch's `ctx`. It outlives
// the watch task (the Watch keeps it alive until after stop+wait), so the
// callbacks can dereference it safely. A single watch sets `handler`; a merged
// watch sets `multi_handler`. The callbacks of one watch never run at the
// same time, so `callback_error` needs no lock.
struct WatchState {
  NotificationHandler* handler = nullptr;
  MultiNotificationHandler* multi_handler = nullptr;
  // Set when a handler callback threw; reported to `on_end` in place of the
  // outcome, since the exception is why the watch stopped.
  std::optional<ErrorInfo> callback_error;
};

// Records the exception being handled as the watch's error. Call only from a
// catch block.
inline void record_callback_error(WatchState& state,
                                  const char* callback) noexcept {
  try {
    std::string what = "an exception not derived from std::exception";
    try {
      throw;
    } catch (const std::exception& error) {
      what = error.what();
    } catch (...) {
      // reason: the type is unknown, so the default description stands.
    }
    ErrorInfo info;
    info.kind = AvisoErrorKind_Internal;
    info.message = std::string("the handler's ") + callback + " threw: " + what;
    state.callback_error = std::move(info);
  } catch (...) {
    // reason: building the message failed (out of memory); the watch still
    // stops, and on_end reports the outcome without the description.
  }
}

// C-ABI trampolines. They translate the C callbacks into virtual calls and
// stop any C++ exception from unwinding across the boundary into Rust.
extern "C" inline bool watch_on_notification(void* ctx,
                                             const AvisoNotification* notification) {
  auto* state = static_cast<WatchState*>(ctx);
  try {
    return state->handler->on_notification(Notification(notification));
  } catch (...) {
    record_callback_error(*state, "on_notification");
    return false;
  }
}

// The error an ended watch reports, if any: a handler exception first, else
// the outcome's error. Takes ownership of `outcome`.
inline std::optional<ErrorInfo> end_error(const WatchState& state,
                                          AvisoOutcome* outcome) {
  OutcomePtr owned(outcome);
  if (state.callback_error) {
    return state.callback_error;
  }
  if (owned && !aviso_outcome_is_ok(owned.get())) {
    return to_error_info(aviso_outcome_error(owned.get()));
  }
  return std::nullopt;
}

extern "C" inline void watch_on_end(void* ctx, AvisoOutcome* outcome) {
  auto* state = static_cast<WatchState*>(ctx);
  try {
    state->handler->on_end(end_error(*state, outcome));
  } catch (...) {
    // reason: on_end is the last callback; nothing runs after it that could
    // report the exception.
  }
}

extern "C" inline bool watch_many_on_notification(
    void* ctx, const char* name, const AvisoNotification* notification) {
  auto* state = static_cast<WatchState*>(ctx);
  try {
    return state->multi_handler->on_notification(std::string(name),
                                                  Notification(notification));
  } catch (...) {
    record_callback_error(*state, "on_notification");
    return false;
  }
}

extern "C" inline bool watch_many_on_error(void* ctx, const char* name,
                                           const AvisoError* error) {
  auto* state = static_cast<WatchState*>(ctx);
  try {
    return state->multi_handler->on_error(std::string(name),
                                          to_error_info(error));
  } catch (...) {
    record_callback_error(*state, "on_error");
    return false;
  }
}

extern "C" inline void watch_many_on_end(void* ctx, AvisoOutcome* outcome) {
  auto* state = static_cast<WatchState*>(ctx);
  try {
    state->multi_handler->on_end(end_error(*state, outcome));
  } catch (...) {
    // reason: on_end is the last callback; nothing runs after it that could
    // report the exception.
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
// is reported through the watch's on_end when it starts. Once attached (or
// moved from), the trigger is used up: calling a setter on it, or attaching it
// again, throws `aviso::Error` (`AvisoErrorKind_InvalidUsage`).
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
    aviso_trigger_set_label(live(), name.c_str());
    return *this;
  }
  Trigger& retries(std::uint32_t retries) {
    aviso_trigger_set_retries(live(), retries);
    return *this;
  }
  Trigger& required(bool required) {
    aviso_trigger_set_required(live(), required);
    return *this;
  }
  Trigger& timeout_secs(std::uint64_t seconds) {
    aviso_trigger_set_timeout_secs(live(), seconds);
    return *this;
  }
  Trigger& fail_fast(bool on) {
    aviso_trigger_set_fail_fast(live(), on);
    return *this;
  }
  Trigger& method(HttpMethod method) {
    aviso_trigger_set_method(live(),
                             static_cast<std::uint32_t>(method));
    return *this;
  }
  Trigger& header(const std::string& name, const std::string& value) {
    aviso_trigger_set_header(live(), name.c_str(), value.c_str());
    return *this;
  }
  Trigger& body_template(const std::string& body) {
    aviso_trigger_set_body_template(live(), body.c_str());
    return *this;
  }
  Trigger& env(const std::string& key, const std::string& value) {
    aviso_trigger_set_env(live(), key.c_str(), value.c_str());
    return *this;
  }
  Trigger& working_dir(const std::string& dir) {
    aviso_trigger_set_working_dir(live(), dir.c_str());
    return *this;
  }

 private:
  friend class WatchRequest;
  explicit Trigger(AvisoTrigger* handle) : handle_(handle) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a trigger");
    }
  }
  // The handle, or throws if this trigger was already attached to a request
  // or moved from.
  [[nodiscard]] AvisoTrigger* live() const {
    if (!handle_) {
      detail::throw_usage(
          "aviso: this Trigger was already used (added to a request or moved "
          "from)");
    }
    return handle_.get();
  }
  AvisoTrigger* release() { return handle_.release(); }
  detail::TriggerPtr handle_;
};

// Fluent builder for a watch request. Defaults to a live watch of `event_type`;
// the `*_from_*` setters add a resume position or switch to replay-only. Once
// consumed by `Client::watch` or `WatchSet::add` (or moved from), the request
// is used up: calling a setter or using it again throws `aviso::Error`
// (`AvisoErrorKind_InvalidUsage`).
class WatchRequest {
 public:
  explicit WatchRequest(const std::string& event_type)
      : handle_(aviso_watch_request_new(event_type.c_str())) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a watch request");
    }
  }

  WatchRequest& filter_json(const std::string& json) {
    aviso_watch_request_set_filter_json(live(), json.c_str());
    return *this;
  }
  WatchRequest& watch_from_sequence(std::uint64_t sequence) {
    aviso_watch_request_watch_from_sequence(live(), sequence);
    return *this;
  }
  WatchRequest& watch_from_date(const std::string& date) {
    aviso_watch_request_watch_from_date(live(), date.c_str());
    return *this;
  }
  WatchRequest& replay_from_sequence(std::uint64_t sequence) {
    aviso_watch_request_replay_from_sequence(live(), sequence);
    return *this;
  }
  WatchRequest& replay_from_date(const std::string& date) {
    aviso_watch_request_replay_from_date(live(), date.c_str());
    return *this;
  }

  // Attaches a trigger, consuming it.
  WatchRequest& add_trigger(Trigger trigger) {
    AvisoWatchRequest* request = live();
    static_cast<void>(trigger.live());
    AvisoTrigger* raw = trigger.release();
    aviso_watch_request_add_trigger(request, &raw);
    return *this;
  }

 private:
  friend class Client;
  friend class WatchSet;
  // The handle, or throws if this request was already consumed by a watch or
  // a WatchSet, or moved from.
  [[nodiscard]] AvisoWatchRequest* live() const {
    if (!handle_) {
      detail::throw_usage(
          "aviso: this WatchRequest was already used (consumed by a watch or a "
          "WatchSet, or moved from)");
    }
    return handle_.get();
  }
  AvisoWatchRequest* release() { return handle_.release(); }
  detail::WatchRequestPtr handle_;
};

// Named watch requests for `Client::watch_many`. Move-only.
class WatchSet {
 public:
  WatchSet() : handle_(aviso_watch_list_new()) {
    if (!handle_) {
      detail::throw_internal("aviso: failed to allocate a watch set");
    }
  }

  // Adds `request` under `name`, consuming it. Names must be non-empty and
  // unique. A mistake, here or in the request, is reported through the
  // handler's `on_end` when the watch starts, and no watch opens. Throws
  // `aviso::Error` (`AvisoErrorKind_InvalidUsage`) if this set was already
  // passed to `Client::watch_many` or `request` was already used.
  WatchSet& add(const std::string& name, WatchRequest& request) {
    if (!handle_) {
      detail::throw_usage("aviso: this WatchSet was already used by watch_many");
    }
    if (!request.handle_) {
      detail::throw_usage("aviso: the WatchRequest for '" + name +
                          "' was already used");
    }
    AvisoWatchRequest* raw_request = request.release();
    aviso_watch_list_add(handle_.get(), name.c_str(), &raw_request);
    return *this;
  }

 private:
  friend class Client;
  AvisoWatchList* release() { return handle_.release(); }
  detail::WatchListPtr handle_;
};

// An RAII watch. Move-only. The destructor stops the watch and waits for it to
// finish (so the handler's `on_end` has returned) before releasing the handle.
// `stop()` and `wait()` on a moved-from watch throw `aviso::Error`
// (`AvisoErrorKind_InvalidUsage`).
class Watch {
 public:
  Watch(Watch&&) noexcept = default;
  Watch(const Watch&) = delete;
  Watch& operator=(const Watch&) = delete;

  // Stops and finishes the watch this object holds, as the destructor does,
  // then takes over `other`'s. The old watch's `on_end` has returned before
  // its state is released, so assigning over a running watch is safe.
  Watch& operator=(Watch&& other) noexcept {
    if (this != &other) {
      finish();
      handle_ = std::move(other.handle_);
      state_ = std::move(other.state_);
    }
    return *this;
  }

  ~Watch() { finish(); }

  // Requests a graceful stop. Nonblocking and idempotent. Throws
  // `aviso::Error` (`AvisoErrorKind_InvalidUsage`) on a moved-from `Watch`.
  void stop() { aviso_watch_stop(live()); }

  // Blocks until the watch has ended and the handler's `on_end` has returned.
  // Throws `aviso::Error` (`AvisoErrorKind_InvalidUsage`) if called from inside
  // a callback or on a moved-from `Watch`.
  void wait() { detail::check(detail::OutcomePtr(aviso_watch_wait(live()))); }

 private:
  friend class Client;
  Watch(AvisoWatch* handle, std::unique_ptr<detail::WatchState> state)
      : handle_(handle), state_(std::move(state)) {}

  // Stops the watch, waits until its `on_end` has returned, and releases the
  // handle and the state the callbacks use. A no-op on a moved-from watch.
  void finish() noexcept {
    if (!handle_) {
      return;
    }
    aviso_watch_stop(handle_.get());
    detail::OutcomePtr drained(aviso_watch_wait(handle_.get()));
    // Only an InvalidUsage outcome means wait was refused (called on a
    // runtime/callback thread) and the task may still call on_end with `ctx`;
    // leak the state then rather than free it out from under a live task. Any
    // other outcome (ok, or a task panic) means the task has ended, so the
    // state is freed.
    if (drained != nullptr && !aviso_outcome_is_ok(drained.get())) {
      const AvisoError* error = aviso_outcome_error(drained.get());
      if (error != nullptr && error->kind == AvisoErrorKind_InvalidUsage) {
        static_cast<void>(state_.release());
      }
    }
    handle_.reset();
    state_.reset();
  }

  // The handle, or throws if this watch was moved from.
  [[nodiscard]] AvisoWatch* live() const {
    if (!handle_) {
      detail::throw_usage("aviso: this Watch was moved from");
    }
    return handle_.get();
  }

  detail::WatchPtr handle_;
  std::unique_ptr<detail::WatchState> state_;
};

inline Watch Client::watch(WatchRequest& request, NotificationHandler& handler) {
  AvisoClient* client = live();
  static_cast<void>(request.live());
  auto state = std::make_unique<detail::WatchState>();
  state->handler = &handler;
  AvisoWatchRequest* raw_request = request.release();
  AvisoWatch* watch = aviso_client_watch(client, &raw_request,
                                         detail::watch_on_notification,
                                         detail::watch_on_end, state.get());
  if (watch == nullptr) {
    detail::throw_internal("aviso: failed to start the watch");
  }
  return Watch(watch, std::move(state));
}

inline Watch Client::watch_many(WatchSet& watches,
                               MultiNotificationHandler& handler) {
  AvisoClient* client = live();
  if (!watches.handle_) {
    detail::throw_usage("aviso: this WatchSet was already used by watch_many");
  }
  auto state = std::make_unique<detail::WatchState>();
  state->multi_handler = &handler;
  AvisoWatchList* raw_list = watches.release();
  AvisoWatch* watch = aviso_client_watch_many(
      client, &raw_list, detail::watch_many_on_notification,
      detail::watch_many_on_error, detail::watch_many_on_end, state.get());
  if (watch == nullptr) {
    detail::throw_internal("aviso: failed to start the watch");
  }
  return Watch(watch, std::move(state));
}

}  // namespace aviso
