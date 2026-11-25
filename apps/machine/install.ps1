# Installs Meerkat Machine for the current Windows user, then runs it with any
# given arguments. Published with each release; the release fills in VERSION.
#
#   & ([scriptblock]::Create((irm https://github.com/what1f/meerkat/releases/download/vVERSION/install.ps1))) pair URL CODE --cert-sha256 HEX
#
# MEERKAT_INSTALL_DIR changes where the binaries go (default %USERPROFILE%\.meerkat\bin).
# MEERKAT_DOWNLOAD_BASE changes where the release files are downloaded from.

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Version = "@VERSION@"
if ($Version -like "*@*") {
  throw "This install script is not from a Meerkat release."
}

$Base = if ($env:MEERKAT_DOWNLOAD_BASE) {
  $env:MEERKAT_DOWNLOAD_BASE.TrimEnd("/")
} else {
  "https://github.com/what1f/meerkat/releases/download/v$Version"
}

$Dir = if ($env:MEERKAT_INSTALL_DIR) {
  $env:MEERKAT_INSTALL_DIR
} else {
  Join-Path (Join-Path $env:USERPROFILE ".meerkat") "bin"
}

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
switch ($Arch) {
  "x64" { $Platform = "windows-x86_64" }
  default { throw "Meerkat Machine is not available for Windows $Arch yet." }
}

$Binaries = @("meerkat-machine.exe", "meerkat.exe")

function Test-Installed {
  foreach ($binary in $Binaries) {
    if (-not (Test-Path -LiteralPath (Join-Path $Dir $binary) -PathType Leaf)) {
      return $false
    }
  }
  try {
    $observed = (& (Join-Path $Dir "meerkat-machine.exe") --version 2>$null | Out-String).Trim()
    return $observed -eq "meerkat-machine $Version"
  } catch {
    return $false
  }
}

function Download-File([string]$Source, [string]$Destination) {
  if ($Source.StartsWith("file://")) {
    $localPath = ([System.Uri]$Source).LocalPath
    Copy-Item -LiteralPath $localPath -Destination $Destination -Force
  } else {
    Invoke-WebRequest -Uri $Source -OutFile $Destination -UseBasicParsing
  }
}

function Quote-WinArg([string]$Value) {
  $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
  $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
  return '"' + $escaped + '"'
}

function Get-DefaultMachineHome {
  return Join-Path $env:USERPROFILE ".meerkat"
}

function Get-NormalizedHomeKey([string]$ExplicitHome) {
  $resolved = if ([string]::IsNullOrEmpty($ExplicitHome)) {
    Get-DefaultMachineHome
  } else {
    $ExplicitHome
  }
  try {
    $resolved = [System.IO.Path]::GetFullPath($resolved)
  } catch {
  }
  return $resolved.TrimEnd('\', '/').ToLowerInvariant()
}

function Split-WinCommandLine([string]$CommandLine) {
  $result = New-Object System.Collections.Generic.List[string]
  if ([string]::IsNullOrWhiteSpace($CommandLine)) {
    return ,$result.ToArray()
  }
  $len = $CommandLine.Length
  $i = 0
  while ($i -lt $len -and ($CommandLine[$i] -eq ' ' -or $CommandLine[$i] -eq "`t")) {
    $i++
  }
  if ($i -ge $len) {
    return ,$result.ToArray()
  }

  # Argv[0]: executable name (quotes delimit, backslashes are literal).
  $prog = New-Object System.Text.StringBuilder
  if ($CommandLine[$i] -eq '"') {
    $i++
    while ($i -lt $len -and $CommandLine[$i] -ne '"') {
      [void]$prog.Append($CommandLine[$i])
      $i++
    }
    if ($i -lt $len -and $CommandLine[$i] -eq '"') {
      $i++
    }
  } else {
    while ($i -lt $len -and $CommandLine[$i] -ne ' ' -and $CommandLine[$i] -ne "`t") {
      [void]$prog.Append($CommandLine[$i])
      $i++
    }
  }
  [void]$result.Add($prog.ToString())

  # Argv[1..N]: Microsoft C/C++ command-line parsing rules.
  while ($i -lt $len) {
    while ($i -lt $len -and ($CommandLine[$i] -eq ' ' -or $CommandLine[$i] -eq "`t")) {
      $i++
    }
    if ($i -ge $len) {
      break
    }
    $arg = New-Object System.Text.StringBuilder
    $inQuotes = $false
    while ($i -lt $len) {
      $ch = $CommandLine[$i]
      if (-not $inQuotes -and ($ch -eq ' ' -or $ch -eq "`t")) {
        break
      }
      if ($ch -eq '\') {
        $slashes = 0
        while ($i -lt $len -and $CommandLine[$i] -eq '\') {
          $slashes++
          $i++
        }
        if ($i -lt $len -and $CommandLine[$i] -eq '"') {
          if ($slashes -ge 2) {
            [void]$arg.Append('\', [int][Math]::Floor($slashes / 2))
          }
          if (($slashes % 2) -eq 1) {
            [void]$arg.Append('"')
            $i++
          } elseif ($inQuotes -and ($i + 1) -lt $len -and $CommandLine[$i + 1] -eq '"') {
            [void]$arg.Append('"')
            $i += 2
          } else {
            $inQuotes = -not $inQuotes
            $i++
          }
        } else {
          [void]$arg.Append('\', $slashes)
        }
      } elseif ($ch -eq '"') {
        if ($inQuotes -and ($i + 1) -lt $len -and $CommandLine[$i + 1] -eq '"') {
          [void]$arg.Append('"')
          $i += 2
        } else {
          $inQuotes = -not $inQuotes
          $i++
        }
      } else {
        [void]$arg.Append($ch)
        $i++
      }
    }
    [void]$result.Add($arg.ToString())
  }
  return ,$result.ToArray()
}

function Parse-MachineArgs([string[]]$ArgList) {
  $explicitHome = $null
  $isPair = $false
  $isForeground = $false
  $isInfoOnly = $false
  $i = 0
  while ($i -lt $ArgList.Count) {
    $token = $ArgList[$i]
    switch ($token) {
      "pair" {
        $isPair = $true
        $i += 3
      }
      "--home" {
        if (($i + 1) -lt $ArgList.Count) {
          $explicitHome = $ArgList[$i + 1]
          $i += 2
        } else {
          $i++
        }
      }
      "--foreground" {
        $isForeground = $true
        $i++
      }
      "--version" {
        $isInfoOnly = $true
        $i++
      }
      "--help" {
        $isInfoOnly = $true
        $i++
      }
      "--cert-sha256" { $i += 2 }
      "--runtime-url" { $i += 2 }
      "--link-secret-file" { $i += 2 }
      "--instance-id" { $i += 2 }
      default { $i++ }
    }
  }
  return [pscustomobject]@{
    ExplicitHome = $explicitHome
    HomeKey      = (Get-NormalizedHomeKey $explicitHome)
    IsPair       = $isPair
    IsForeground = $isForeground
    IsInfoOnly   = $isInfoOnly
  }
}

function Extract-HomeEntryFromCommandLine([string]$CommandLine) {
  [string[]]$argv = Split-WinCommandLine $CommandLine
  [string[]]$argList = if ($argv.Count -gt 1) { @($argv[1..($argv.Count - 1)]) } else { @() }
  $parsed = Parse-MachineArgs $argList
  return [pscustomobject]@{
    ExplicitHome = $parsed.ExplicitHome
    Key          = $parsed.HomeKey
  }
}

function Stop-InstalledMachine {
  $targetExe = [System.IO.Path]::GetFullPath((Join-Path $Dir "meerkat-machine.exe"))
  $stoppedHomes = New-Object System.Collections.Generic.List[object]
  $seenKeys = New-Object System.Collections.Generic.HashSet[string]
  $procs = Get-CimInstance Win32_Process -Filter "Name = 'meerkat-machine.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.ExecutablePath -and ([System.IO.Path]::GetFullPath($_.ExecutablePath) -ieq $targetExe) }
  foreach ($proc in $procs) {
    $entry = Extract-HomeEntryFromCommandLine $proc.CommandLine
    if ($seenKeys.Add($entry.Key)) {
      [void]$stoppedHomes.Add($entry)
    }
    Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
  }
  if ($stoppedHomes.Count -gt 0) {
    $deadline = (Get-Date).AddSeconds(5)
    while ((Get-Date) -lt $deadline) {
      $still = Get-CimInstance Win32_Process -Filter "Name = 'meerkat-machine.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and ([System.IO.Path]::GetFullPath($_.ExecutablePath) -ieq $targetExe) }
      if (-not $still) { break }
      Start-Sleep -Milliseconds 100
    }
  }
  return ,$stoppedHomes.ToArray()
}

function Start-BackgroundMachines([object[]]$Homes) {
  $exePath = Join-Path $Dir "meerkat-machine.exe"
  foreach ($entry in $Homes) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exePath
    if (-not [string]::IsNullOrEmpty($entry.ExplicitHome)) {
      $psi.Arguments = "--home " + (Quote-WinArg $entry.ExplicitHome)
    }
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $proc = [System.Diagnostics.Process]::Start($psi)
    if (-not $proc) {
      $label = if ([string]::IsNullOrEmpty($entry.ExplicitHome)) { Get-DefaultMachineHome } else { $entry.ExplicitHome }
      throw "Could not restart Meerkat Machine for $label."
    }
  }
}

[object[]]$stoppedHomes = @()

if (-not (Test-Installed)) {
  $ArchiveName = "meerkat-machine-$Version-$Platform"
  $TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("meerkat-install-" + [System.Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Path $TempDir -Force | Out-Null
  $backupDir = Join-Path $TempDir "backup"
  try {
    Write-Output "Installing Meerkat Machine $Version..."
    $ZipPath = Join-Path $TempDir "$ArchiveName.zip"
    $ShaPath = Join-Path $TempDir "$ArchiveName.zip.sha256"
    Download-File "$Base/$ArchiveName.zip" $ZipPath
    Download-File "$Base/$ArchiveName.zip.sha256" $ShaPath

    $Expected = ((Get-Content -LiteralPath $ShaPath -Raw).Trim() -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
      throw "The download was damaged. Run the command again."
    }

    $ExtractDir = Join-Path $TempDir "extracted"
    Expand-Archive -LiteralPath $ZipPath -DestinationPath $ExtractDir -Force
    $PackageRoot = Join-Path $ExtractDir $ArchiveName
    foreach ($binary in $Binaries) {
      if (-not (Test-Path -LiteralPath (Join-Path $PackageRoot $binary) -PathType Leaf)) {
        throw "Downloaded archive is missing $binary."
      }
    }

    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    New-Item -ItemType Directory -Path $backupDir -Force | Out-Null

    $stoppedHomes = Stop-InstalledMachine

    $toInstall = @($Binaries)

    try {
      foreach ($binary in $toInstall) {
        $existing = Join-Path $Dir $binary
        if (Test-Path -LiteralPath $existing -PathType Leaf) {
          Copy-Item -LiteralPath $existing -Destination (Join-Path $backupDir $binary) -Force
        }
      }
      foreach ($binary in $toInstall) {
        Copy-Item -LiteralPath (Join-Path $PackageRoot $binary) -Destination (Join-Path $Dir $binary) -Force
      }
      if (-not (Test-Installed)) {
        throw "Meerkat Machine $Version does not run on this computer."
      }
    } catch {
      foreach ($binary in $toInstall) {
        $bak = Join-Path $backupDir $binary
        if (Test-Path -LiteralPath $bak -PathType Leaf) {
          Copy-Item -LiteralPath $bak -Destination (Join-Path $Dir $binary) -Force -ErrorAction SilentlyContinue
        }
      }
      if ($stoppedHomes.Count -gt 0) {
        try { Start-BackgroundMachines $stoppedHomes } catch {}
      }
      throw
    }
  } finally {
    Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
  }

  if ($args.Count -eq 0 -and $stoppedHomes.Count -gt 0) {
    Start-BackgroundMachines $stoppedHomes
  }
}

if ($args.Count -gt 0) {
  $parsedArgs = Parse-MachineArgs ([string[]]$args)
  [object[]]$otherHomes = @()
  [object[]]$targetHomes = @()
  if ($stoppedHomes.Count -gt 0) {
    $otherHomes = @($stoppedHomes | Where-Object { $_.Key -ne $parsedArgs.HomeKey })
    $targetHomes = @($stoppedHomes | Where-Object { $_.Key -eq $parsedArgs.HomeKey })
    if ($otherHomes.Count -gt 0) {
      try { Start-BackgroundMachines $otherHomes } catch {}
    }
  }
  & (Join-Path $Dir "meerkat-machine.exe") @args
  $code = $LASTEXITCODE
  if ($targetHomes.Count -gt 0) {
    $tookOverTarget = ($code -eq 0) -and $parsedArgs.IsPair -and (-not $parsedArgs.IsForeground) -and (-not $parsedArgs.IsInfoOnly)
    if (-not $tookOverTarget) {
      try { Start-BackgroundMachines $targetHomes } catch {}
    }
  }
  if ($PSCommandPath) {
    exit $code
  }
  $global:LASTEXITCODE = $code
  return
}

Write-Output "Meerkat Machine $Version is installed in $Dir"
