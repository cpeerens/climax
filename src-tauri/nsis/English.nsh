; Climax installer strings - overrides Tauri's generated English.nsh.
;
; WHY THIS FILE EXISTS. Tauri's stock copy is dangerous for an app whose whole
; value is a database the user cannot get back:
;   - `olderOrUnknownVersionInstalled` RECOMMENDED uninstalling first, which is
;     the path that leads to the delete-app-data checkbox. It recommended the
;     destructive route.
;   - `deleteAppData` read "Delete the application data", which does not tell
;     anyone that ticking it destroys every session, every cumshot and every
;     setting, permanently.
;   - Neither maintenance option said what happens to the library, so a person
;     updating had to guess.
;
; The RADIO ORDER is fixed by Tauri's template ("uninstall before installing" is
; first, and NSIS checks the first radio), and the template is compiled into the
; CLI binary, so it cannot be reordered without forking it. The labels therefore
; carry the steer instead: the safe option is marked recommended, and the page
; body states outright that the library survives either way. That leaves the
; delete checkbox as the only route to data loss, and it is now unmissable.
;
; MUST list EVERY string Tauri defines - this file REPLACES the generated one,
; so an omission is a build error. Keep in step when upgrading the Tauri CLI
; (taken from 2.11.2). Ordinary hyphens only, per the project copy rule.

LangString addOrReinstall ${LANG_ENGLISH} "Repair or change the installation"
LangString alreadyInstalled ${LANG_ENGLISH} "Climax is already installed"
LangString alreadyInstalledLong ${LANG_ENGLISH} "${PRODUCTNAME} ${VERSION} is already installed. Choose what to do, then click Next. Your library is kept either way."
LangString appRunning ${LANG_ENGLISH} "{{product_name}} is running. Please close it first, then try again."
LangString appRunningOkKill ${LANG_ENGLISH} "{{product_name}} is running.$\nClick OK to close it."
LangString chooseMaintenanceOption ${LANG_ENGLISH} "Choose what you want to do."
LangString choowHowToInstall ${LANG_ENGLISH} "Choose how to install ${PRODUCTNAME}."
LangString createDesktop ${LANG_ENGLISH} "Create desktop shortcut"
LangString dontUninstall ${LANG_ENGLISH} "Update the existing installation (recommended)"
LangString dontUninstallDowngrade ${LANG_ENGLISH} "Install over the top (disabled when going back to an older version)"
LangString failedToKillApp ${LANG_ENGLISH} "Couldn't close {{product_name}}. Please close it yourself, then try again."
LangString installingWebview2 ${LANG_ENGLISH} "Installing WebView2..."
LangString newerVersionInstalled ${LANG_ENGLISH} "A newer version of ${PRODUCTNAME} is already installed. Going back to an older version isn't recommended. Your library is kept either way, but an older version may not be able to read data written by a newer one. Choose what to do, then click Next."
LangString older ${LANG_ENGLISH} "older"
LangString olderOrUnknownVersionInstalled ${LANG_ENGLISH} "An $R4 version of ${PRODUCTNAME} is installed. Updating over the top is the normal choice and keeps everything. Your sessions, history and settings are kept either way - they live outside the program folder. Choose what to do, then click Next."
LangString silentDowngrades ${LANG_ENGLISH} "Going back to an older version is disabled for the silent installer. Please use the normal installer instead.$\n"
LangString unableToUninstall ${LANG_ENGLISH} "Couldn't uninstall."
LangString uninstallApp ${LANG_ENGLISH} "Uninstall ${PRODUCTNAME}"
LangString uninstallBeforeInstalling ${LANG_ENGLISH} "Remove the old version first, then install"
LangString unknown ${LANG_ENGLISH} "unknown"
LangString webview2AbortError ${LANG_ENGLISH} "Couldn't install WebView2. Climax can't run without it. Try restarting the installer."
LangString webview2DownloadError ${LANG_ENGLISH} "Error: downloading WebView2 failed - $0"
LangString webview2DownloadSuccess ${LANG_ENGLISH} "WebView2 downloaded"
LangString webview2Downloading ${LANG_ENGLISH} "Downloading WebView2..."
LangString webview2InstallError ${LANG_ENGLISH} "Error: installing WebView2 failed with exit code $1"
LangString webview2InstallSuccess ${LANG_ENGLISH} "WebView2 installed"
LangString deleteAppData ${LANG_ENGLISH} "Also delete my Climax library - every session, all history and all settings. This cannot be undone."
