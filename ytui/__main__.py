import sys

from . import sources
from .app import main

_BROWSER_FLAGS = {f"--{name}" for name in sources._COOKIE_BROWSERS if name}

if __name__ == "__main__":
    for arg in sys.argv[1:]:
        if arg in _BROWSER_FLAGS:
            sources.set_cookie_browser(arg.removeprefix("--"))
    main()
