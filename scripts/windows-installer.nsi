Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "VERSION must be defined"
!endif
!ifndef SOURCE_DIR
  !error "SOURCE_DIR must be defined"
!endif
!ifndef ICON_PATH
  !error "ICON_PATH must be defined"
!endif
!ifndef OUTPUT_FILE
  !error "OUTPUT_FILE must be defined"
!endif
!ifndef INSTALL_LIST
  !error "INSTALL_LIST must be defined"
!endif
!ifndef UNINSTALL_LIST
  !error "UNINSTALL_LIST must be defined"
!endif
!ifndef STOP_BINARIES
  !error "STOP_BINARIES must be defined"
!endif

Name "Meerkat ${VERSION}"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Meerkat"
InstallDirRegKey HKCU "Software\Meerkat" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "${ICON_PATH}"
!define MUI_UNICON "${ICON_PATH}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\meerkat-desktop.exe"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

!macro StopInstalledProcesses
  ; Stop only the exact executables bundled by this installer inside $INSTDIR.
  System::Call 'Kernel32::SetEnvironmentVariable(t "MEERKAT_INSTDIR", t "$INSTDIR")'
  System::Call 'Kernel32::SetEnvironmentVariable(t "MEERKAT_STOP_BINARIES", t "${STOP_BINARIES}")'
  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -Command "$dir = [IO.Path]::GetFullPath($env:MEERKAT_INSTDIR); $targets = ($env:MEERKAT_STOP_BINARIES -split \",\") | Where-Object { $_ } | ForEach-Object { [IO.Path]::GetFullPath((Join-Path $dir $_)) }; $procs = Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and ($targets -contains [IO.Path]::GetFullPath($_.ExecutablePath)) }; foreach ($p in $procs) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }; $deadline = (Get-Date).AddSeconds(5); while ((Get-Date) -lt $deadline) { $alive = Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and ($targets -contains [IO.Path]::GetFullPath($_.ExecutablePath)) }; if (-not $alive) { break }; Start-Sleep -Milliseconds 100 }"'
!macroend

!macro CheckWebView2 outVar
  StrCpy ${outVar} ""
  ReadRegStr ${outVar} HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} ${outVar} == ""
  ${OrIf} ${outVar} == "0.0.0.0"
    ReadRegStr ${outVar} HKCU "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} ${outVar} == ""
  ${OrIf} ${outVar} == "0.0.0.0"
    ReadRegStr ${outVar} HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} ${outVar} == "0.0.0.0"
    StrCpy ${outVar} ""
  ${EndIf}
!macroend

Section "WebView2 Runtime" SecWebView2
  SectionIn RO
  !insertmacro CheckWebView2 $0
  ${If} $0 == ""
    DetailPrint "Installing Microsoft Edge WebView2 Runtime..."
    nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -WindowStyle Hidden -Command "$ErrorActionPreference = \"Stop\"; $p = Join-Path $env:TEMP \"MicrosoftEdgeWebview2Setup.exe\"; Invoke-WebRequest -Uri \"https://go.microsoft.com/fwlink/p/?LinkId=2124703\" -OutFile $p -UseBasicParsing; $proc = Start-Process -FilePath $p -ArgumentList \"/silent /install\" -Wait -PassThru; Remove-Item $p -Force -ErrorAction SilentlyContinue; exit $proc.ExitCode"'
    Pop $1
    ${If} $1 != 0
      Abort "Microsoft Edge WebView2 Runtime is required to run Meerkat."
    ${EndIf}
    !insertmacro CheckWebView2 $0
    ${If} $0 == ""
      Abort "Microsoft Edge WebView2 Runtime is required to run Meerkat."
    ${EndIf}
  ${EndIf}
SectionEnd

Section "Meerkat" SecMain
  SectionIn RO

  !insertmacro StopInstalledProcesses

  !include "${INSTALL_LIST}"

  SetOutPath "$INSTDIR"

  WriteRegStr HKCU "Software\Meerkat" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "DisplayName" "Meerkat"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "Publisher" "Meerkat Contributors"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "DisplayIcon" "$INSTDIR\meerkat-desktop.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat" "NoRepair" 1

  CreateDirectory "$SMPROGRAMS\Meerkat"
  CreateShortcut "$SMPROGRAMS\Meerkat\Meerkat.lnk" "$INSTDIR\meerkat-desktop.exe" "" "$INSTDIR\meerkat-desktop.exe" 0
  CreateShortcut "$SMPROGRAMS\Meerkat\Uninstall Meerkat.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\Meerkat.lnk" "$INSTDIR\meerkat-desktop.exe" "" "$INSTDIR\meerkat-desktop.exe" 0

  WriteUninstaller "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Uninstall"
  !insertmacro StopInstalledProcesses

  Delete "$DESKTOP\Meerkat.lnk"
  Delete "$SMPROGRAMS\Meerkat\Meerkat.lnk"
  Delete "$SMPROGRAMS\Meerkat\Uninstall Meerkat.lnk"
  RMDir "$SMPROGRAMS\Meerkat"

  ; Delete only the exact files installed by this package; never use RMDir /r.
  !include "${UNINSTALL_LIST}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Meerkat"
  DeleteRegKey HKCU "Software\Meerkat"
SectionEnd
