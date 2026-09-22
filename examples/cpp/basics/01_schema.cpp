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
// uses it.

#include "../common.hpp"

#include <iostream>
#include <string>

int main() {
  std::cout << "client version " << aviso::version() << '\n';
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
    switch (error.error().kind) {
      case AvisoErrorKind_Config:
        // No usable server address. Not a bug in this program.
        std::cerr << "no usable server address: set AVISO_BASE_URL, or put "
                     "base_url in ~/.config/aviso/config.yaml\n";
        return 0;
      case AvisoErrorKind_Transport:
        // The server is not there. Also not a bug in this program.
        return 0;
      default:
        return 1;
    }
  }
}
