[CmdletBinding()]
param(
    [string]$Port = "",
    [ValidateRange(0, 86400)]
    [int]$DurationSeconds = 0,
    [string]$OutputDirectory = "",
    [switch]$ListPorts,
    [switch]$SelfTest
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"

$script:ToolVersion = "1.0.0"
$script:BaudRate = 115200
$script:RedlinePatterns = @(
    "psram init fail",
    "PSRAM trim is corrupted",
    "This chip has no psram",
    "flash init fail",
    "RF init FAIL",
    "queue alloc FAIL",
    "OTA: FAIL",
    "OTA task alloc FAIL",
    "key=missing",
    "sig=DEV-OPTIONAL",
    "audio: FAIL",
    "mic_queue create failed",
    "bt_enable failed",
    "stack init timeout",
    "stack init failed",
    "BT init failed",
    "control server register failed",
    "interrupt server register failed",
    "SDP server register failed"
)

function Get-AvailableSerialPorts {
    return @([System.IO.Ports.SerialPort]::GetPortNames() | Sort-Object)
}

function Get-Ch340PortCandidates {
    $result = New-Object System.Collections.Generic.List[string]

    try {
        $cimPorts = @(Get-CimInstance Win32_SerialPort -ErrorAction Stop | Where-Object {
            $_.PNPDeviceID -match "VID_1A86&PID_7523" -or
            $_.Name -match "CH340|USB-SERIAL"
        })
        foreach ($item in $cimPorts) {
            if ($item.DeviceID -and -not $result.Contains([string]$item.DeviceID)) {
                $result.Add([string]$item.DeviceID)
            }
        }
    }
    catch {
        # Win32_SerialPort may require elevation on managed Windows hosts.
    }

    try {
        $pnpPorts = @(Get-PnpDevice -Class Ports -PresentOnly -ErrorAction Stop | Where-Object {
            $_.InstanceId -match "VID_1A86&PID_7523" -or
            $_.FriendlyName -match "CH340|USB-SERIAL"
        })
        foreach ($item in $pnpPorts) {
            if ($item.FriendlyName -match "\((COM[0-9]+)\)") {
                $name = [string]$Matches[1]
                if (-not $result.Contains($name)) {
                    $result.Add($name)
                }
            }
        }
    }
    catch {
        # Fall back to the serial-port list below.
    }

    return @($result | Sort-Object)
}

function Resolve-DiagnosticPort {
    param([string]$RequestedPort)

    $available = @(Get-AvailableSerialPorts)
    if ($RequestedPort) {
        $normalized = $RequestedPort.ToUpperInvariant()
        if ($available -notcontains $normalized) {
            throw "Serial port $normalized is not available. Present ports: $($available -join ', ')"
        }
        return $normalized
    }

    $ch340 = @(Get-Ch340PortCandidates | Where-Object { $available -contains $_ })
    if ($ch340.Count -eq 1) {
        return $ch340[0]
    }
    if ($ch340.Count -gt 1) {
        throw "Multiple CH340 ports were found: $($ch340 -join ', '). Pass -Port explicitly."
    }

    $usbLike = @($available | Where-Object { $_ -ne "COM1" })
    if ($usbLike.Count -eq 1) {
        Write-Warning "CH340 identity was not readable; using the only non-COM1 port $($usbLike[0])."
        return $usbLike[0]
    }

    throw "AI-M61 CH340 was not found. Present ports: $($available -join ', '). Pass -Port COMx explicitly."
}

function Protect-DiagnosticLine {
    param([AllowEmptyString()][string]$Line)

    $safe = $Line
    $safe = [regex]::Replace(
        $safe,
        "(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b",
        "<BD_ADDR>"
    )
    $safe = [regex]::Replace(
        $safe,
        "(?i)(\[USB-INIT\]\s+Serial:\s*)\S+",
        '${1}<USB_SERIAL>'
    )
    $safe = [regex]::Replace(
        $safe,
        "(?i)(passkey(?:\s*[:=]\s*|\s+))\d{6}\b",
        '${1}<PASSKEY>'
    )
    $safe = [regex]::Replace(
        $safe,
        "(?i)(\b(?:conn|chan|mic_q|enc|dec)\s*=?)0x[0-9a-f]{6,16}\b",
        '${1}<POINTER>'
    )
    return $safe
}

function Get-DiagnosticCategory {
    param([string]$Line)
    if ($Line -match "^\s*\[([^\]]+)\]") {
        return [string]$Matches[1]
    }
    if ($Line -match "component_version|Version of used components") {
        return "VERSION"
    }
    if ($Line -match "psram|dynamic memory|ocram") {
        return "MEMORY"
    }
    return "SYSTEM"
}

function Test-Redline {
    param([string]$Line)
    foreach ($pattern in $script:RedlinePatterns) {
        if ($Line.IndexOf($pattern, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
            return $pattern
        }
    }
    return $null
}

function Invoke-SelfTest {
    $mac = Protect-DiagnosticLine "[BT] BD_ADDR: B4:0E:CF:2A:FF:E0"
    $serial = Protect-DiagnosticLine "[USB-INIT] Serial: B40ECF2AFFDF0000"
    $passkey = Protect-DiagnosticLine "[BT] SSP confirm passkey: 123456"
    $pointer = Protect-DiagnosticLine "[AUDIO] mic_q=0x62fe80ec"

    if ($mac -notmatch "<BD_ADDR>") { throw "BD_ADDR redaction failed" }
    if ($serial -notmatch "<USB_SERIAL>") { throw "USB serial redaction failed" }
    if ($passkey -notmatch "<PASSKEY>") { throw "Passkey redaction failed" }
    if ($pointer -notmatch "<POINTER>") { throw "Pointer redaction failed" }
    if ((Test-Redline "[B] OTA: FAIL") -ne "OTA: FAIL") { throw "Redline detection failed" }
    if (Test-Redline "[B] OTA: OK") { throw "False-positive redline" }
    if ((Get-DiagnosticCategory "[USB-EVT] CONFIGURED") -ne "USB-EVT") {
        throw "Category parsing failed"
    }

    Write-Output "m61-diagnostics self-test passed"
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

if ($ListPorts) {
    $available = @(Get-AvailableSerialPorts)
    $ch340 = @(Get-Ch340PortCandidates)
    [pscustomobject]@{
        AvailablePorts = $available -join ", "
        Ch340Candidates = $ch340 -join ", "
    } | Format-List
    exit 0
}

$selectedPort = Resolve-DiagnosticPort -RequestedPort $Port

if (-not $OutputDirectory) {
    $repositoryRoot = Split-Path -Parent $PSScriptRoot
    $OutputDirectory = Join-Path $repositoryRoot ".device-verify"
}
$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
[System.IO.Directory]::CreateDirectory($outputRoot) | Out-Null

$sessionStamp = Get-Date -Format "yyyyMMdd-HHmmss"
$sessionName = "m61-diagnostics-$sessionStamp-$PID"
$sessionDirectory = Join-Path $outputRoot $sessionName
if (Test-Path -LiteralPath $sessionDirectory) {
    throw "Diagnostic session directory already exists: $sessionDirectory"
}
[System.IO.Directory]::CreateDirectory($sessionDirectory) | Out-Null

$rawLogPath = Join-Path $sessionDirectory "uart.raw.log"
$redactedLogPath = Join-Path $sessionDirectory "uart.redacted.log"
$eventPath = Join-Path $sessionDirectory "events.redacted.jsonl"
$metadataPath = Join-Path $sessionDirectory "metadata.json"
$summaryPath = Join-Path $sessionDirectory "summary.json"
$safeZipPath = Join-Path $outputRoot "$sessionName-safe.zip"

$startedUtc = [DateTime]::UtcNow
$metadata = [ordered]@{
    schema = "ds5dongle.m61-diagnostics"
    schemaVersion = 1
    toolVersion = $script:ToolVersion
    startedUtc = $startedUtc.ToString("o")
    port = $selectedPort
    serial = [ordered]@{
        baudRate = $script:BaudRate
        dataBits = 8
        parity = "None"
        stopBits = 1
        flowControl = "None"
        dtr = $false
        rts = $false
        receiveOnly = $true
    }
    requestedDurationSeconds = $DurationSeconds
    host = [ordered]@{
        osVersion = [System.Environment]::OSVersion.VersionString
        powershell = $PSVersionTable.PSVersion.ToString()
    }
    privacy = [ordered]@{
        rawLogContainsSensitiveIdentifiers = $true
        safeArchiveExcludesRawLog = $true
        redactedFields = @("Bluetooth address", "USB serial", "SSP passkey", "selected pointers")
    }
}
$metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $metadataPath -Encoding UTF8

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$rawWriter = New-Object System.IO.StreamWriter($rawLogPath, $false, $utf8NoBom)
$redactedWriter = New-Object System.IO.StreamWriter($redactedLogPath, $false, $utf8NoBom)
$eventWriter = New-Object System.IO.StreamWriter($eventPath, $false, $utf8NoBom)
$rawWriter.AutoFlush = $false
$redactedWriter.AutoFlush = $false
$eventWriter.AutoFlush = $false

$serialPort = New-Object System.IO.Ports.SerialPort(
    $selectedPort,
    $script:BaudRate,
    [System.IO.Ports.Parity]::None,
    8,
    [System.IO.Ports.StopBits]::One
)
$serialPort.Handshake = [System.IO.Ports.Handshake]::None
$serialPort.DtrEnable = $false
$serialPort.RtsEnable = $false
$serialPort.Encoding = $utf8NoBom

$lineBuffer = New-Object System.Text.StringBuilder
$bytesCaptured = [uint64]0
$linesCaptured = [uint64]0
$redlineMatches = New-Object System.Collections.Generic.List[object]
$categoryCounts = @{}
$observed = [ordered]@{
    firmwareVersion = $null
    sdkCommit = $null
    boot2Version = $null
    board = $null
    psram = $null
    otaReady = $false
    audioReady = $false
    bluetoothReady = $false
    usbReady = $false
    usbConfigured = $false
    controllerConnected = $false
    firstInputReceived = $false
    lastState = $null
}

function Add-DiagnosticLine {
    param([AllowEmptyString()][string]$Line)

    $script:linesCaptured++
    $redacted = Protect-DiagnosticLine $Line
    $category = Get-DiagnosticCategory $Line
    if (-not $script:categoryCounts.ContainsKey($category)) {
        $script:categoryCounts[$category] = 0
    }
    $script:categoryCounts[$category]++

    $redline = Test-Redline $Line
    $severity = if ($redline) { "error" } elseif ($Line -match "(?i)warn|empty|use default") { "warning" } else { "info" }
    if ($redline) {
        $script:redlineMatches.Add([ordered]@{
            line = $script:linesCaptured
            pattern = $redline
            message = $redacted
        })
    }

    if ($Line -match "component_version_sdk:\s*([^\s]+)(?:\s+([^\s]+))?") {
        $script:observed.firmwareVersion = [string]$Matches[1]
        if ($Matches.Count -gt 2) { $script:observed.sdkCommit = [string]$Matches[2] }
    }
    if ($Line -match "component_version_boot2:\s*([^\s]+)") { $script:observed.boot2Version = [string]$Matches[1] }
    if ($Line -match "\[MAIN\]\s+Board:\s*(.+)$") { $script:observed.board = [string]$Matches[1] }
    if ($Line -match "psram_info\s+(.+)$") { $script:observed.psram = [string]$Matches[1] }
    if ($Line -match "\[B\]\s+OTA:\s+OK") { $script:observed.otaReady = $true }
    if ($Line -match "\[B\]\s+audio:\s+OK") { $script:observed.audioReady = $true }
    if ($Line -match "\[BT\]\s+Bluetooth fully initialized") { $script:observed.bluetoothReady = $true }
    if ($Line -match "USB Ready") { $script:observed.usbReady = $true }
    if ($Line -match "\[USB-EVT\]\s+CONFIGURED") { $script:observed.usbConfigured = $true }
    if ($Line -match "\[MAIN\]\s+State:\s+(.+)$") {
        $script:observed.lastState = [string]$Matches[1]
        if ($Matches[1] -match "^CONNECTED") { $script:observed.controllerConnected = $true }
    }
    if ($Line -match "First input report received") { $script:observed.firstInputReceived = $true }

    $script:redactedWriter.WriteLine($redacted)
    $event = [ordered]@{
        hostUtc = [DateTime]::UtcNow.ToString("o")
        line = $script:linesCaptured
        category = $category
        severity = $severity
        message = $redacted
    }
    $script:eventWriter.WriteLine(($event | ConvertTo-Json -Compress))
    Write-Host $redacted
}

Write-Host "AI-M61 receive-only diagnostics"
Write-Host "Port: $selectedPort @ $($script:BaudRate) 8-N-1, no flow control"
Write-Host "Session: $sessionDirectory"
Write-Host "Open the listener first, then press RESET without holding BOOT."
if ($DurationSeconds -eq 0) {
    Write-Host "Press Ctrl+C to stop."
}
else {
    Write-Host "Capture will stop after $DurationSeconds seconds."
}

$serialOpened = $false
$captureFailure = $null
$lastFlushUtc = [DateTime]::UtcNow
try {
    $serialPort.Open()
    $serialOpened = $true
    while ($true) {
        $chunk = $serialPort.ReadExisting()
        if ($chunk.Length -gt 0) {
            $rawWriter.Write($chunk)
            $bytesCaptured += [uint64]$utf8NoBom.GetByteCount($chunk)
            [void]$lineBuffer.Append($chunk)

            $bufferText = $lineBuffer.ToString()
            $newlineIndex = $bufferText.IndexOf("`n", [System.StringComparison]::Ordinal)
            while ($newlineIndex -ge 0) {
                $line = $bufferText.Substring(0, $newlineIndex).TrimEnd("`r")
                Add-DiagnosticLine $line
                $bufferText = $bufferText.Substring($newlineIndex + 1)
                $newlineIndex = $bufferText.IndexOf("`n", [System.StringComparison]::Ordinal)
            }
            [void]$lineBuffer.Clear()
            [void]$lineBuffer.Append($bufferText)
        }

        $nowUtc = [DateTime]::UtcNow
        if (($nowUtc - $lastFlushUtc).TotalSeconds -ge 1) {
            $rawWriter.Flush()
            $redactedWriter.Flush()
            $eventWriter.Flush()
            $lastFlushUtc = $nowUtc
        }
        if ($DurationSeconds -gt 0 -and ($nowUtc - $startedUtc).TotalSeconds -ge $DurationSeconds) {
            break
        }
        Start-Sleep -Milliseconds 20
    }
}
catch {
    $captureFailure = $_.Exception.Message
    throw
}
finally {
    if ($lineBuffer.Length -gt 0) {
        Add-DiagnosticLine $lineBuffer.ToString().TrimEnd("`r")
    }
    if ($serialOpened -and $serialPort.IsOpen) {
        $serialPort.Close()
    }
    $serialPort.Dispose()
    $rawWriter.Flush()
    $redactedWriter.Flush()
    $eventWriter.Flush()
    $rawWriter.Dispose()
    $redactedWriter.Dispose()
    $eventWriter.Dispose()

    $endedUtc = [DateTime]::UtcNow
    $summary = [ordered]@{
        schema = "ds5dongle.m61-diagnostics.summary"
        schemaVersion = 1
        startedUtc = $startedUtc.ToString("o")
        endedUtc = $endedUtc.ToString("o")
        durationSeconds = [math]::Round(($endedUtc - $startedUtc).TotalSeconds, 3)
        bytesCaptured = $bytesCaptured
        linesCaptured = $linesCaptured
        captureFailure = $captureFailure
        redlineCount = $redlineMatches.Count
        redlines = $redlineMatches.ToArray()
        categoryCounts = $categoryCounts
        observed = $observed
        files = [ordered]@{
            raw = "uart.raw.log"
            redacted = "uart.redacted.log"
            events = "events.redacted.jsonl"
            metadata = "metadata.json"
        }
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding UTF8

    try {
        Compress-Archive -LiteralPath @($metadataPath, $summaryPath, $redactedLogPath, $eventPath) -DestinationPath $safeZipPath -CompressionLevel Optimal
    }
    catch {
        Write-Warning "Could not create the safe archive: $($_.Exception.Message)"
    }

    Write-Host ""
    Write-Host "Capture finished: $sessionDirectory"
    Write-Host "Lines: $linesCaptured; bytes: $bytesCaptured; redlines: $($redlineMatches.Count)"
    if (Test-Path -LiteralPath $safeZipPath) {
        Write-Host "Share-safe archive: $safeZipPath"
    }
    Write-Warning "uart.raw.log is intentionally excluded from the safe archive because it may contain identifiers."
}
