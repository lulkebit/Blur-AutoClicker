# v2.1.0 - 24.02.2026 (d/m/y)
## 🔺 Fix:
- CPU logging does not report 0.0% value if no samples are made (previously lowered average cpu usage artificially)
- CPU logging frequency is dynamic, making the averages more accurate at lower runtimes.

# v2.1.0 - 23.02.2026 (d/m/y)
## ❇️ New Features:
- Added Opt-in Telemetry Popup (Honestly didn't wanna do that but EU laws and stuff :3)
- measure and log cpu usage
- Changed Data collection from Google to Supabase
- Moved ENTIE backend to Rust for better peformance
- logging of clicker session time and total time, session clicks and total clicks
- click status has a greer outline while active, making it more obvious that the auto clicker has been turned on.

## 🔹 Changed:
- Keybind field automatically unfocuses so that it doesn't bug out when you instantly try to activate the autoclicker without removing focus from the field.
- Refractor of:
     - main.py
     - settings_manager.py
     - hotkey_manager.py
     - rust_translation.py
- updated file structure
- Config.ini now saves at %appdata%/blur009/autoclicker/config.ini
- split up main.py into individual files to reduce line count per file.
- ReadMe Updated

## 🔺 Fix:

## 🔸 Performance Updates:
- switching to rust massively increased performance, dropping cpu usage by ceveral percent. (down to ~1%avg during use on my system)

## 🪦 Removed:
- Switch to Go was good, but I realized after way too much debugging that syscall took 84% of my runtime performance. So, to Rust we go.. (Go was basically talking to itsself over and over to do the clicks, while Rust is doing it directly, which is why the performance increase is so big).


# v2.0.0 - 18.02.2026 (d/m/y)
## ❇️ New Features:
- Added On / Off hint next to the shortcut field.
- Added smoothing to the mouse movement to combat the "teleporting" of the cursor.
- Added an Offset Chance button that makes the Click Offset only happen sometimes.
- Added Anonymous Telemetry to find the most common settings people use/don't use
- Added Info about Telemetry and support options in Program Settings
- Added an Advanced Options button that makes the gui simpler for ppl who need a simple auto clicker :3

## 🔹 Changed:
- Changed the UI to be less complex and more user-friendly (I hope).
- Changed UI to adjust to the window size when enabling/disabling Advanced options (took 4ever)
- Increased Click Speed cap to different values depending on the selected time frame (second, minute, hour, day). It is not recommended to use speeds over 500 even though it is technically possible.
- Renamed Scripts folder to src
- Split some UI and Settings features into settings_manager.py to clean up main.py
- very sneaky shark emoji hidden somewhere in the code. You get a cookie if you find it.

## 🔺 Fix:
- Fixed the Offset to apply in the radius of a circle instead of a square around the set position  
(not really a "bug" but this is the way I wanted it to work when I thought of the feature).

## 🔸 Performance Updates:
- Introduced click batching at higher cps to send multiple clicks every call. This allows for more clicks than before because windows pointer resolution was limiting the amount of calls that the clicker was able to make.
- Variables are initialized outside the isRunning loop
- more that I probably forgot because I've been sitting here for 10h making this work :3


# v2.0.0 - 18.02.2026 (d/m/y) (just an empty template for me here)
## ❇️ New Features:

## 🔹 Changed:

## 🔺 Fix:

## 🔸 Performance Updates:

## 🪦 Removed:

