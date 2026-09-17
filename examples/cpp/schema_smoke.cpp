// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

// Smoke test for the binding: print the version, connect, and call a blocking
// verb. Connection settings come from the environment (see aviso_env.hpp).
//
// With no server reachable the only expected failure is a transport error, so
// CI runs this without a server and treats a transport error as success; any
// other error fails.

#include "aviso_env.hpp"

#include <iostream>
#include <string>

int main() {
  std::cout << "aviso version: " << aviso::version() << '\n';
  std::cout << "connecting to " << example::base_url() << '\n';

  try {
    aviso::Client client = example::connect();
    const std::string schema = client.schema();
    std::cout << "schema: " << schema << '\n';
    return 0;
  } catch (const aviso::Error& error) {
    const aviso::ErrorInfo& info = error.error();
    std::cerr << "aviso error (" << static_cast<int>(info.kind)
              << "): " << error.what() << '\n';
    return info.kind == AvisoErrorKind_Transport ? 0 : 1;
  }
}
