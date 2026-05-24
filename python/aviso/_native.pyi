"""Type stubs for the PyO3 extension module `aviso._native`.

Hand-written. Users never import this module directly; the public Python
surface lives in `aviso.__init__`. These stubs describe the compiled
extension's exports so `ty check` can resolve imports from the wrapper
package.
"""

from __future__ import annotations

VERSION: str
