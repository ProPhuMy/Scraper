function Install-Scraper {
    $ErrorActionPreference = 'Stop'

    # Platform check
    if ($null -ne $IsWindows -and -not $IsWindows) {
        Write-Host "scraper-install: on Linux/macOS use install.sh" -ForegroundColor Red
        return
    }

    $repo = "ProPhuMy/Scraper"
    $asset = "scraper-windows-x86_64.exe"

    # Options via environment variables
    $version = $env:SCRAPER_VERSION
    $installDir = if ($env:SCRAPER_INSTALL_DIR) { $env:SCRAPER_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }

    if ($version) {
        $url = "https://github.com/$repo/releases/download/$version/$asset"
        $checksumUrl = "https://github.com/$repo/releases/download/$version/${asset}.sha256"
    } else {
        $url = "https://github.com/$repo/releases/latest/download/$asset"
        $checksumUrl = "https://github.com/$repo/releases/latest/download/${asset}.sha256"
    }

    # Dry run mode
    if ($env:SCRAPER_DRY_RUN -eq '1') {
        Write-Host "asset: $asset"
        Write-Host "url: $url"
        return
    }

    Write-Host "scraper-install: detected windows x86_64"

    # PowerShell 5.1 TLS & Progress Preference
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $origProgressPreference = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'

    $tempDir = $null
    $envKey = $null
    try {
        # Create fresh temp directory with GUID
        $tempGuid = [Guid]::NewGuid().ToString()
        $tempDir = Join-Path ([IO.Path]::GetTempPath()) "scraper-install-$tempGuid"
        # -Path, not -LiteralPath: New-Item has no -LiteralPath in Windows PowerShell 5.1 (GUID path has no wildcards)
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

        $downloadExe = Join-Path $tempDir $asset
        $downloadSha = Join-Path $tempDir "${asset}.sha256"

        $versionLabel = if ($version) { $version } else { "latest" }
        Write-Host "scraper-install: downloading $asset ($versionLabel)..."

        try {
            Invoke-WebRequest -Uri $url -OutFile $downloadExe -UseBasicParsing
        } catch {
            Write-Host "scraper-install: failed to download asset from ${url}: $($_.Exception.Message)" -ForegroundColor Red
            return
        }

        try {
            Invoke-WebRequest -Uri $checksumUrl -OutFile $downloadSha -UseBasicParsing
        } catch {
            Write-Host "scraper-install: failed to download checksum from ${checksumUrl}: $($_.Exception.Message)" -ForegroundColor Red
            return
        }

        # Checksum verification
        Write-Host "scraper-install: verifying checksum..."
        $shaContent = Get-Content -LiteralPath $downloadSha -Raw
        $expectedHash = ($shaContent.Trim() -split '\s+')[0]
        $actualHash = (Get-FileHash -LiteralPath $downloadExe -Algorithm SHA256).Hash

        if ([string]::IsNullOrWhiteSpace($expectedHash) -or -not $actualHash.Equals($expectedHash, [System.StringComparison]::OrdinalIgnoreCase)) {
            Write-Host "scraper-install: checksum verification failed for $asset" -ForegroundColor Red
            return
        }
        Write-Host "scraper-install: checksum ok"

        # Install binary
        try {
            if (-not (Test-Path -LiteralPath $installDir)) {
                New-Item -ItemType Directory -Path $installDir -Force | Out-Null
            }
        } catch {
            Write-Host "scraper-install: failed to create directory ${installDir}: $($_.Exception.Message)" -ForegroundColor Red
            return
        }

        $targetExe = Join-Path $installDir "scraper.exe"
        try {
            Move-Item -LiteralPath $downloadExe -Destination $targetExe -Force
        } catch {
            Write-Host "scraper-install: could not replace ${targetExe}: $($_.Exception.Message). Close any running scraper and try again" -ForegroundColor Red
            return
        }

        # PATH configuration
        if ($env:SCRAPER_NO_MODIFY_PATH -ne '1') {
            $normInstallDir = $installDir.Trim().TrimEnd('\', '/')
            # Read the raw registry value: [Environment]::GetEnvironmentVariable would expand
            # %VARS% and writing that back would freeze them (e.g. %JAVA_HOME%in).
            $envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
            $userPath = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $userEntries = if ($userPath) { $userPath -split ';' } else { @() }

            $alreadyOnPath = $false
            foreach ($entry in $userEntries) {
                $normEntry = $entry.Trim().TrimEnd('\', '/')
                if ($normEntry.Length -gt 0 -and $normEntry.Equals($normInstallDir, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $alreadyOnPath = $true
                    break
                }
            }

            if ($alreadyOnPath) {
                Write-Host "scraper-install: $installDir already on PATH"
            } else {
                $newUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
                    $installDir
                } else {
                    if ($userPath.TrimEnd().EndsWith(';')) {
                        "$($userPath.TrimEnd())$installDir"
                    } else {
                        "$userPath;$installDir"
                    }
                }
                $envKey.SetValue('Path', $newUserPath, [Microsoft.Win32.RegistryValueKind]::ExpandString)
                # Writing the registry directly doesn't notify Windows; setting (then clearing) a dummy
                # user variable broadcasts WM_SETTINGCHANGE so new terminals see the updated PATH.
                [Environment]::SetEnvironmentVariable('SCRAPER_INSTALL_REFRESH', '1', [System.EnvironmentVariableTarget]::User)
                [Environment]::SetEnvironmentVariable('SCRAPER_INSTALL_REFRESH', $null, [System.EnvironmentVariableTarget]::User)
                Write-Host "scraper-install: added $installDir to your PATH (open a new terminal elsewhere)"
            }

            # Update current session $env:Path if not already present
            $sessionEntries = if ($env:Path) { $env:Path -split ';' } else { @() }
            $sessionHasIt = $false
            foreach ($entry in $sessionEntries) {
                $normEntry = $entry.Trim().TrimEnd('\', '/')
                if ($normEntry.Length -gt 0 -and $normEntry.Equals($normInstallDir, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $sessionHasIt = $true
                    break
                }
            }
            if (-not $sessionHasIt) {
                if ([string]::IsNullOrWhiteSpace($env:Path)) {
                    $env:Path = $installDir
                } elseif ($env:Path.TrimEnd().EndsWith(';')) {
                    $env:Path = "$($env:Path.TrimEnd())$installDir"
                } else {
                    $env:Path = "$env:Path;$installDir"
                }
            }
        }

        # Smoke test
        try {
            & $targetExe --help *>$null
            if ($LASTEXITCODE -ne 0) {
                Write-Host "scraper-install: WARNING: '$targetExe --help' failed. The binary may not run on this system." -ForegroundColor Yellow
            }
        } catch {
            Write-Host "scraper-install: WARNING: '$targetExe --help' failed. The binary may not run on this system." -ForegroundColor Yellow
        }

        # Next steps output
        Write-Host "scraper-install: installed to $targetExe"
        Write-Host ""
        Write-Host "Next steps:"
        Write-Host "  1. Run 'scraper init' (creates your config folder with scraper.toml and .env)"
        Write-Host "  2. Edit [profile] in the scraper.toml it prints"
        Write-Host "  3. Run 'scraper sync', then 'scraper scan', then 'scraper browse'"
    } catch {
        Write-Host "scraper-install: $($_.Exception.Message)" -ForegroundColor Red
        return
    } finally {
        if ($envKey) { $envKey.Close() }
        if ($tempDir -and (Test-Path -LiteralPath $tempDir)) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        $ProgressPreference = $origProgressPreference
    }
}

Install-Scraper
