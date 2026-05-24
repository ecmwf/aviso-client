// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

(() => {
    const darkThemes = ['ayu', 'navy', 'coal'];
    const lightThemes = ['light', 'rust'];

    const classList = document.getElementsByTagName('html')[0].classList;

    let lastThemeWasLight = true;
    for (const cssClass of classList) {
        if (darkThemes.includes(cssClass)) {
            lastThemeWasLight = false;
            break;
        }
    }

    const theme = lastThemeWasLight ? 'default' : 'dark';
    if (typeof mermaid !== 'undefined') {
        mermaid.initialize({ startOnLoad: true, theme });
    }

    // Simplest way to make mermaid re-render the diagrams in the new theme is via refreshing the page.
    // Each theme button ID may be absent on pages that lack the mdbook theme picker (search results,
    // 404 page, custom layouts), so the lookup is guarded; otherwise a missing element throws on
    // addEventListener and breaks the rest of the page JS.
    const reloadIfThemeChanged = (expectedLight) => () => {
        if (lastThemeWasLight === expectedLight) {
            window.location.reload();
        }
    };

    for (const darkTheme of darkThemes) {
        const btn = document.getElementById(darkTheme);
        if (btn) {
            btn.addEventListener('click', reloadIfThemeChanged(true));
        }
    }

    for (const lightTheme of lightThemes) {
        const btn = document.getElementById(lightTheme);
        if (btn) {
            btn.addEventListener('click', reloadIfThemeChanged(false));
        }
    }
})();
