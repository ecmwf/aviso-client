# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Shared pytest configuration for the pyaviso Python suite.

Tests run against the locally-built `pyaviso._native` extension produced by
``uv run maturin develop``. The conftest stays minimal until later
commits introduce fixtures.
"""

from __future__ import annotations
