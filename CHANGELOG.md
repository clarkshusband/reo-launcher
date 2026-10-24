## 2.8.1 (2026-10-24)

### Fixes

- Fixed the browser replaying its page animation and losing the picked cluster when switching cluster or type
- Fixed CurseForge jar identification, loader detection, category filters and version lists past the first 50 by [clarkshusband](https://github.com/clarkshusband)
- Fixed large downloads failing after 10 minutes and the game console stopping on non-UTF-8 log lines by [clarkshusband](https://github.com/clarkshusband)
- Fixed too long names overflowing in the drag and drop window
- Fixed filters resetting when you change a tab in cluster view
- Fixed Play staying disabled after a database restore, and Windows updates now ask before restarting by [clarkshusband](https://github.com/clarkshusband)
- Added Mojang's log4j hardening for Minecraft 1.7 to 1.16.5 by [clarkshusband](https://github.com/clarkshusband)
- Added ctrl + f shortcut for focusing the searchbar in logs and live log views
- Fixed sort by not applying to OneClient versions in the instance list

## 2.8.0 (2026-10-09)

### Features

- Added safe multiple minecraft instances launching, only dedicated clusters tho by [rozwader](https://github.com/rozwader)

### Fixes

- Added better informations about errors from mclogs. by [rozwader](https://github.com/rozwader)
- Added status bar showing up if a player is logged out or he has a stale refresh token by [rozwader](https://github.com/rozwader)
- Added a filter for choosing modpack loaders by [rozwader](https://github.com/rozwader)
- SkyBlock content installed without asking is removed from 26.3 clusters once, and turning an optional bundle down removes its mods
- Fixed shield.io badges not rendering properly in markdown viewers by [rozwader](https://github.com/rozwader)

## 2.7.0 (2026-10-05)

### Features

- Code blocks are selectable, and the open link confirmation has a copy button by [LynithDev](https://github.com/LynithDev)
- Game settings have a Game Arguments field for passing extra arguments to Minecraft, globally or per cluster by [Wyvest](https://github.com/Wyvest)
- Game logs hide account tokens, and the game output starts with the launch command ready to paste into a terminal by [LynithDev](https://github.com/LynithDev)
- GitHub hosted mods have a "View in browser" option that opens their repository by [LynithDev](https://github.com/LynithDev)
- The changelog page now shows each GitHub release's notes by [LynithDev](https://github.com/LynithDev)
- The mods folder now syncs with the launcher: jars added, renamed or replaced by hand show up in the Mods list, and jars deleted by hand are switched off instead of being put back by [Wyvest](https://github.com/Wyvest)

- Choose which GPU the game renders on by [LynithDev](https://github.com/LynithDev)

### Fixes

- Icons show on Linux systems with a comma decimal locale by [LynithDev](https://github.com/LynithDev)
- The "Uploaded to mclo.gs" notification no longer repeats when switching back to the Logs tab by [LynithDev](https://github.com/LynithDev)
- Open folder and open in browser work again on Linux by [LynithDev](https://github.com/LynithDev)
- Optional bundles are now offered before the first launch of every new version instead of being installed or skipped silently by [Wyvest](https://github.com/Wyvest)
- System information in the settings sidebar shows a tooltip explaining it can be clicked to copy by [LynithDev](https://github.com/LynithDev)
