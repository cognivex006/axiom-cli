#requires -version 5.1
<#
    Shared helpers for the axiom-cli Windows x64 distribution helpers.

    Owner: axiom-cli.

    This file is dot-sourced by Install-AxiomCli.ps1, Uninstall-AxiomCli.ps1 and the
    Windows harness. It is deliberately written for the Windows shell that ships with
    the OS (Windows PowerShell 5.1) as well as PowerShell 7: no Bash, no WSL, no
    Docker, no elevation, no P/Invoke, no symlink and no junction.

    Everything here writes JSON through one canonical writer so that the same
    transaction produces byte-identical documents on Windows PowerShell 5.1 and on
    PowerShell 7. That matters because approval is bound to a sha256 over the plan
    document: a shell that formatted JSON differently would produce a different
    digest for the same plan.

    Nothing in this file reads from the network, interpolates a shell string or
    mutates a machine-wide setting.
#>

$script:AxiomSpecVersion = '2.0.0-draft.1'
$script:AxiomPlatformId = 'windows-x64'
$script:AxiomEntrypoint = 'axiom-cli.exe'
$script:AxiomUserRegistryKey = 'HKCU:\Environment'
$script:AxiomUserPathValueName = 'Path'

# ---------------------------------------------------------------------------
# Canonical JSON
# ---------------------------------------------------------------------------

function ConvertTo-AxiomJsonString {
    param([AllowNull()][string]$Text)

    if ($null -eq $Text) { return '' }

    $builder = New-Object System.Text.StringBuilder
    foreach ($ch in $Text.ToCharArray()) {
        $code = [int]$ch
        if ($code -eq 34) { [void]$builder.Append('\"') }
        elseif ($code -eq 92) { [void]$builder.Append('\\') }
        elseif ($code -eq 10) { [void]$builder.Append('\n') }
        elseif ($code -eq 13) { [void]$builder.Append('\r') }
        elseif ($code -eq 9) { [void]$builder.Append('\t') }
        elseif ($code -eq 8) { [void]$builder.Append('\b') }
        elseif ($code -eq 12) { [void]$builder.Append('\f') }
        elseif ($code -lt 32) { [void]$builder.Append('\u' + $code.ToString('x4')) }
        else { [void]$builder.Append($ch) }
    }
    return $builder.ToString()
}

<#
    Serialize a value to canonical JSON text: two-space indent, LF newlines, keys in
    insertion order (use [ordered]@{} to control order), invariant number formatting.
#>
function ConvertTo-AxiomJson {
    param(
        [AllowNull()]$Value,
        [int]$Depth = 0
    )

    $pad = ' ' * (2 * $Depth)
    $inner = ' ' * (2 * ($Depth + 1))

    if ($null -eq $Value) { return 'null' }
    if ($Value -is [bool]) { if ($Value) { return 'true' } else { return 'false' } }
    if ($Value -is [string]) { return '"' + (ConvertTo-AxiomJsonString -Text $Value) + '"' }
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [int16] -or $Value -is [byte] `
        -or $Value -is [double] -or $Value -is [single] -or $Value -is [decimal]) {
        return [string]::Format([System.Globalization.CultureInfo]::InvariantCulture, '{0}', $Value)
    }

    if ($Value -is [System.Collections.IDictionary]) {
        $keys = @($Value.Keys)
        if ($keys.Count -eq 0) { return '{}' }
        $parts = New-Object System.Collections.Generic.List[string]
        foreach ($key in $keys) {
            $parts.Add($inner + '"' + (ConvertTo-AxiomJsonString -Text ([string]$key)) + '": ' +
                (ConvertTo-AxiomJson -Value $Value[$key] -Depth ($Depth + 1)))
        }
        return '{' + "`n" + ($parts -join ("," + "`n")) + "`n" + $pad + '}'
    }

    if ($Value -is [System.Collections.IEnumerable]) {
        $items = @($Value)
        if ($items.Count -eq 0) { return '[]' }
        $parts = New-Object System.Collections.Generic.List[string]
        foreach ($item in $items) {
            $parts.Add($inner + (ConvertTo-AxiomJson -Value $item -Depth ($Depth + 1)))
        }
        return '[' + "`n" + ($parts -join ("," + "`n")) + "`n" + $pad + ']'
    }

    return '"' + (ConvertTo-AxiomJsonString -Text ([string]$Value)) + '"'
}

function Write-AxiomJsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [AllowNull()]$Value
    )

    $directory = Split-Path -Parent $Path
    if ($directory -and -not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, (ConvertTo-AxiomJson -Value $Value) + "`n", $encoding)
}

function ConvertTo-AxiomJsonText {
    param([AllowNull()]$Value)
    return (ConvertTo-AxiomJson -Value $Value) + "`n"
}

function Read-AxiomJsonFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    try {
        return (ConvertFrom-Json -InputObject ([System.IO.File]::ReadAllText($Path)))
    } catch {
        return $null
    }
}

# ---------------------------------------------------------------------------
# Hashing and host facts
# ---------------------------------------------------------------------------

function Get-AxiomSha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-AxiomSha256HexOfText {
    param([AllowNull()][string]$Text)
    if ($null -eq $Text) { $Text = '' }
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = (New-Object System.Text.UTF8Encoding($false)).GetBytes($Text)
        return ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function Get-AxiomTimestamp {
    return (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ',
        [System.Globalization.CultureInfo]::InvariantCulture)
}

function New-AxiomRequestId {
    $ticks = [System.DateTime]::UtcNow.Ticks
    return ('axiom-cli-windows-' + $PID + '-' + $ticks.ToString('x'))
}

function New-AxiomTransactionId {
    $stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmss', [System.Globalization.CultureInfo]::InvariantCulture)
    $suffix = ([guid]::NewGuid().ToString('n')).Substring(0, 8)
    return ("txn-$stamp-$suffix")
}

function Test-AxiomSessionElevated {
    try {
        $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = New-Object System.Security.Principal.WindowsPrincipal($identity)
        return $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        return $false
    }
}

function Get-AxiomHostInfo {
    $arch = 'unknown'
    switch ($env:PROCESSOR_ARCHITECTURE) {
        'AMD64' { $arch = 'x86_64' }
        'ARM64' { $arch = 'aarch64' }
        'x86' { $arch = 'x86' }
        default { $arch = [string]$env:PROCESSOR_ARCHITECTURE }
    }

    $windowsVersion = 'unknown'
    try {
        $os = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop
        $windowsVersion = ([string]$os.Caption).Trim() + ' ' + ([string]$os.Version).Trim()
    } catch {
        $windowsVersion = ([string][Environment]::OSVersion.Version).Trim()
    }

    return [ordered]@{
        os                  = 'windows'
        arch                = $arch
        windows_version     = $windowsVersion
        powershell_version  = $PSVersionTable.PSVersion.ToString()
        session_elevated    = (Test-AxiomSessionElevated)
    }
}

# ---------------------------------------------------------------------------
# Diagnostics and the install-result envelope
# ---------------------------------------------------------------------------

function Write-AxiomDiag {
    param(
        [string]$Message = '',
        [switch]$Visible
    )
    # Diagnostics always go to stderr so that `-Json` writes exactly one JSON object
    # to stdout, matching the CLI rule in the distribution contract section 2.
    [Console]::Error.WriteLine($Message)
}

function New-AxiomEnvelopeBase {
    param([Parameter(Mandatory = $true)][string]$Operation)

    return [ordered]@{
        schema_version                = 1
        spec_version                  = $script:AxiomSpecVersion
        envelope_kind                 = 'install-result'
        operation                     = $Operation
        outcome                       = 'refused'
        exit_code                     = 8
        status                        = 'error'
        message                       = ''
        retryable                     = $false
        platform                      = $script:AxiomPlatformId
        host                          = (Get-AxiomHostInfo)
        install_root                  = $null
        bin_dir                       = $null
        dry_run                       = $true
        mutated                       = $false
        plan_digest                   = $null
        approved_digest               = $null
        transaction_id                = $null
        interrupted_install_recovered = $false
        release_set                   = $null
        artifacts                     = @()
        unverified_artifacts          = @()
        components                    = @()
        refusals                      = @()
        path_rule                     = [ordered]@{
            scope               = 'none'
            registry_key        = 'HKCU\Environment'
            value_name          = 'Path'
            entry               = ''
            applied             = $false
            machine_wide_change = $false
            previous_present    = $false
        }
        service_registration          = [ordered]@{
            kind        = 'none'
            task_name   = $null
            owner       = $null
            registered  = $false
            removed     = $false
            reason      = 'no managed-service registration is recorded by this envelope'
        }
        preserved                     = @()
        removed                       = @()
        elevation_required            = $false
        shell                         = 'windows-powershell'
        limitations                   = @()
        generated_at                  = (Get-AxiomTimestamp)
        request_id                    = (New-AxiomRequestId)
    }
}

function Add-AxiomRefusal {
    param(
        [Parameter(Mandatory = $true)]$Envelope,
        [Parameter(Mandatory = $true)][string]$Check,
        [string]$Artifact = '-',
        [AllowNull()][string]$Expected = $null,
        [AllowNull()][string]$Actual = $null,
        [Parameter(Mandatory = $true)][string]$Reason
    )

    $entry = [ordered]@{
        check           = $Check
        artifact        = $Artifact
        expected_sha256 = $Expected
        actual_sha256   = $Actual
        reason          = $Reason
    }
    $Envelope.refusals = @($Envelope.refusals) + @($entry)
}

function Complete-AxiomEnvelope {
    param(
        [Parameter(Mandatory = $true)]$Envelope,
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][string]$Outcome,
        [Parameter(Mandatory = $true)][string]$Status,
        [Parameter(Mandatory = $true)][string]$Message,
        [bool]$Retryable = $false,
        [AllowNull()][string]$JsonOutPath = $null,
        [switch]$EmitJson
    )

    $Envelope.exit_code = $ExitCode
    $Envelope.outcome = $Outcome
    $Envelope.status = $Status
    $Envelope.message = $Message
    $Envelope.retryable = $Retryable

    if ($JsonOutPath) { Write-AxiomJsonFile -Path $JsonOutPath -Value $Envelope }

    if ($EmitJson) {
        [Console]::Out.WriteLine((ConvertTo-AxiomJson -Value $Envelope))
    } else {
        Write-AxiomDiag ("axiom-cli windows install-result: operation={0} outcome={1} exit_code={2} message={3}" -f `
            $Envelope.operation, $Outcome, $ExitCode, $Message)
    }
    return $ExitCode
}

# ---------------------------------------------------------------------------
# Per-user PATH rule (user scope only; never machine scope)
# ---------------------------------------------------------------------------

function Get-AxiomUserPathSnapshot {
    $result = [ordered]@{ present = $false; value = ''; kind = 'ExpandString' }
    try {
        $item = Get-Item -LiteralPath $script:AxiomUserRegistryKey -ErrorAction Stop
        $raw = $item.GetValue($script:AxiomUserPathValueName, $null, 'DoNotExpandEnvironmentNames')
        if ($null -ne $raw) {
            $result.present = $true
            $result.value = [string]$raw
            $kind = $item.GetValueKind($script:AxiomUserPathValueName)
            if ([string]$kind -eq 'String') { $result.kind = 'String' } else { $result.kind = 'ExpandString' }
        }
    } catch {
        # A missing HKCU:\Environment is not an error: the rule creates it.
    }
    return $result
}

function Set-AxiomUserPathValue {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value,
        [string]$Kind = 'ExpandString'
    )

    if (-not (Test-Path -LiteralPath $script:AxiomUserRegistryKey)) {
        New-Item -Path $script:AxiomUserRegistryKey -Force | Out-Null
    }
    $type = 'ExpandString'
    if ($Kind -eq 'String') { $type = 'String' }
    Set-ItemProperty -LiteralPath $script:AxiomUserRegistryKey -Name $script:AxiomUserPathValueName `
        -Value $Value -Type $type
}

function Get-AxiomMachinePathSnapshot {
    # Read-only. Used as evidence that an install changed nothing machine-wide.
    try {
        $item = Get-Item -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment' -ErrorAction Stop
        $raw = $item.GetValue('Path', $null, 'DoNotExpandEnvironmentNames')
        return [string]$raw
    } catch {
        return ''
    }
}

function Split-AxiomPathList {
    param([AllowNull()][string]$Value)
    if ([string]::IsNullOrEmpty($Value)) { return @() }
    return @($Value -split ';')
}

function Test-AxiomPathContains {
    param(
        [AllowNull()][string]$Value,
        [Parameter(Mandatory = $true)][string]$Entry
    )
    foreach ($token in (Split-AxiomPathList -Value $Value)) {
        if ($token.Trim().TrimEnd('\') -ieq $Entry.Trim().TrimEnd('\')) { return $true }
    }
    return $false
}

function Add-AxiomPathEntry {
    param(
        [AllowNull()][string]$Value,
        [Parameter(Mandatory = $true)][string]$Entry
    )
    if (Test-AxiomPathContains -Value $Value -Entry $Entry) { return $Value }
    if ([string]::IsNullOrEmpty($Value)) { return $Entry }
    return $Value.TrimEnd(';') + ';' + $Entry
}

function Remove-AxiomPathEntry {
    param(
        [AllowNull()][string]$Value,
        [Parameter(Mandatory = $true)][string]$Entry
    )
    if ([string]::IsNullOrEmpty($Value)) { return $Value }
    $kept = New-Object System.Collections.Generic.List[string]
    foreach ($token in (Split-AxiomPathList -Value $Value)) {
        if ($token.Trim().TrimEnd('\') -ieq $Entry.Trim().TrimEnd('\')) { continue }
        $kept.Add($token)
    }
    return ($kept -join ';')
}

# ---------------------------------------------------------------------------
# Transaction lock
# ---------------------------------------------------------------------------

function Enter-AxiomTransactionLock {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$TransactionId
    )

    $directory = Split-Path -Parent $Path
    if ($directory -and -not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }

    for ($attempt = 0; $attempt -lt 2; $attempt++) {
        try {
            $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
            $payload = ConvertTo-AxiomJson -Value ([ordered]@{
                pid            = $PID
                transaction_id = $TransactionId
                acquired_at    = (Get-AxiomTimestamp)
            })
            $bytes = (New-Object System.Text.UTF8Encoding($false)).GetBytes($payload)
            $stream.Write($bytes, 0, $bytes.Length)
            $stream.Flush()
            return $stream
        } catch [System.IO.IOException] {
            # The lock exists. It is stale only when its recorded process is gone.
            $holder = Read-AxiomJsonFile -Path $Path
            $holderAlive = $false
            if ($holder -and $holder.pid) {
                $holderAlive = [bool](Get-Process -Id ([int]$holder.pid) -ErrorAction SilentlyContinue)
            }
            if (-not $holderAlive) {
                Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
                continue
            }
            return $null
        }
    }
    return $null
}

function Exit-AxiomTransactionLock {
    param(
        $Stream,
        [Parameter(Mandatory = $true)][string]$Path
    )
    if ($Stream) {
        try { $Stream.Dispose() } catch { }
    }
    Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
# Release-set loading
# ---------------------------------------------------------------------------

function Resolve-AxiomReleaseSetPath {
    param([Parameter(Mandatory = $true)][string]$ReleaseSet)

    if (Test-Path -LiteralPath $ReleaseSet -PathType Container) {
        return (Join-Path $ReleaseSet 'release-set.json')
    }
    return $ReleaseSet
}

function Get-AxiomDefaultInstallRoot {
    $local = $env:LOCALAPPDATA
    if ([string]::IsNullOrEmpty($local)) {
        $local = Join-Path $env:USERPROFILE 'AppData\Local'
    }
    return (Join-Path $local 'Axiom')
}

function Get-AxiomPathEntryForRoot {
    param(
        [Parameter(Mandatory = $true)][string]$InstallRoot,
        [Parameter(Mandatory = $true)][string]$BinDir
    )
    # The default root is a fixed per-user location, so it is recorded with its
    # environment reference and stays correct if the profile moves. A caller-supplied
    # root is recorded literally.
    if ($InstallRoot.TrimEnd('\') -ieq (Get-AxiomDefaultInstallRoot).TrimEnd('\')) {
        return '%LOCALAPPDATA%\Axiom\bin'
    }
    return $BinDir
}

function Format-AxiomBytes {
    param([long]$Bytes)
    return ("{0:N0}" -f $Bytes)
}
