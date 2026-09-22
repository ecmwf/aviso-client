// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Ask the server what it knows about, before publishing or listening.
//
// The schema catalog lists every event type and, for each, the identifier
// fields it accepts. It is the first call to make against an unfamiliar
// server. On the e2e stack it needs no credentials.
//
// Expect: the version, the catalog, and the schema for one event type.
//
// With no server configured, or none reachable, it says so and still exits
// 0. That makes it a build check that needs nothing running, which is how CI
// uses it. A config file that exists but is wrong is a real error, exit 1.

#include "../common.hpp"

#include <filesystem>
#include <iostream>
#include <string>

namespace {

// True when nothing on this machine can supply a server address: no
// AVISO_BASE_URL, and no config file where from_file() would look. The file
// is ~/.config/aviso/config.yaml unless AVISO_CLIENT_CONFIG_FILE names one.
bool nothing_configured() {
  if (example::env("AVISO_BASE_URL")) {
    return false;
  }
  if (const auto named = example::env("AVISO_CLIENT_CONFIG_FILE")) {
    return !std::filesystem::exists(*named);
  }
  const auto home = example::env("HOME");
  return !home || !std::filesystem::exists(*home + "/.config/aviso/config.yaml");
}

}  // namespace

int main() {
  std::cout << "client version " << aviso::version() << '\n';
  if (nothing_configured()) {
    // Not a failure of this program, so exit 0. Everything below assumes an
    // address exists somewhere, and then any config error is a real one,
    // such as a file that will not parse.
    std::cout << "no server configured: set AVISO_BASE_URL, or put base_url in "
                 "~/.config/aviso/config.yaml\n";
    return 0;
  }
  try {
    aviso::Client client = example::connect();

    // The catalog: every event type and its schema, as one JSON document.
    const std::string catalog = client.schema();
    std::cout << "catalog:\n" << catalog << "\n\n";

    // One event type on its own, which is what you want when writing code
    // that publishes to it: this tells you which identifier fields exist.
    const std::string one = client.schema_for(example::kEventType);
    std::cout << example::kEventType << ":\n" << one << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    example::report(error);
    // A server that is not there is the one failure this example tolerates.
    return error.error().kind == AvisoErrorKind_Transport ? 0 : 1;
  }
}
