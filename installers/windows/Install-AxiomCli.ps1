#requires -version 5.1
<#
.SYNOPSIS
    Install or repair the axiom-cli Windows x64 distribution into a per-user location.

.DESCRIPTION
    Owner: axiom-cli. Windows x64 is the finish-first distribution target and this
    script is its installer.

    It consumes a release set produced by packaging/windows/Build-ReleaseSet.ps1: a directory
    holding the distributed entrypoint `axiom-cli.exe`, any component artifacts this release
    actually carries, and `release-set.json`. The release-set document is the plan, and the
    sha256 of that document is the approval digest:

        $set = & .\packaging\windows\Build-ReleaseSet.ps1 -OutDir .\out\win-x64 -Build
        & .\installers\windows\Install-AxiomCli.ps1 -ReleaseSet $set.ReleaseSetPath -Apply `
            -ApproveDigest $set.PlanDigest

    Properties of this installer, each one a requirement of
    contracts/axiom-cli-distribution-contract.md:

      * Per-user only. The install root defaults to `%LOCALAPPDATA%\Axiom` (the ecosystem
        AXIOM_HOME) and only the HKCU user `Path` value is written. Nothing machine-wide is
        touched; the machine PATH is snapshotted before and after purely as evidence that it
        did not change.
      * No elevation. It never requests or needs an administrator token and behaves identically
        from an ordinary user session.
      * No Bash, WSL, Docker, Node.js or compiler. Everything is Windows PowerShell.
      * Transactional. A journal is written before the first mutation, a failure inside the
        commit rolls back from the recorded pre-image, and an interrupted run is recovered on
        the next run instead of leaving a half-installed tree.
      * Digest-gated. Every artifact is size-checked and sha256-checked before it is used; a
        mismatch aborts the transaction. An artifact this release does not carry is reported
        under `unverified_artifacts` and is never reported as installed.
      * Non-interactive. Approval comes only from `-ApproveDigest`; `-Apply` without a valid
        approval digest is refused. No prompt is ever issued.

    Exit codes follow the canonical CLI vocabulary owned by axiom-specs
    docs/16-CLI-AND-CONTROL-API.md section 6, which the distribution contract references:

        0  success / planned / already installed
        2  validation (missing or malformed approval digest or release set)
        3  release set or artifact not found
        5  approval refused (plan digest mismatch)
        6  conflict (an unowned file occupies the entrypoint path)
        8  I/O or internal error (transaction rolled back)
        9  incompatible (artifact size/digest mismatch, or a refused downgrade)
       10  lock unavailable (another transaction holds the lock; retryable)

    `-Json` writes exactly one JSON object - the install-result envelope described by
    packaging/install-result.schema.json - to stdout; every diagnostic goes to stderr.

.PARAMETER ReleaseSet
    Path to a release-set directory, or directly to its `release-set.json`.

.PARAMETER Apply
    Commit the transaction. Without it the run only plans and verifies, mutates nothing and
    exits 0 with outcome `planned`.

.PARAMETER ApproveDigest
    The sha256 of the release-set document, exactly as Build-ReleaseSet.ps1 printed it.
    Required with `-Apply`; a mismatch is refused with exit 5.

.PARAMETER InstallRoot
    Per-user install root. Defaults to `%LOCALAPPDATA%\Axiom`.

.PARAMETER Force
    Replace an unowned file that already occupies the entrypoint path.

.PARAMETER AllowDowngrade
    Permit installing a release whose version is lower than the installed one.

.PARAMETER NonInteractive
    Never prompt. Accepted so a caller can state the intent explicitly; this installer never
    prompts regardless.

.PARAMETER Json
    Emit exactly one JSON object on stdout.

.PARAMETER Out
    Also write the install-result envelope to this file.

.EXAMPLE
    pwsh -File .\installers\windows\Install-AxiomCli.ps1 -ReleaseSet .\out\win-x64 -Apply `
        -ApproveDigest <sha256> -Json

.NOTES
    The installation engine, the daemon service lifecycle and the per-component update
    transaction stay owned by axiom-graphd. This installer wraps them and never re-implements
    them; where an owner artifact is not built yet the run records that honestly under
    `unverified_artifacts` instead of inventing a version.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ReleaseSet,
    [switch]$Apply,
    [string]$ApproveDigest,
    [string]$InstallRoot,
    [switch]$Force,
    [switch]$AllowDowngrade,
    [switch]$NonInteractive,
    [switch]$Json,
    [string]$Out
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

. (Join-Path $PSScriptRoot 'AxiomCli.Windows.Common.ps1')

# Canonical exit vocabulary (axiom-specs docs/16-CLI-AND-CONTROL-API.md section 6).
$exitOk = 0
$exitValidation = 2
$exitNotFound = 3
$exitAuthorization = 5
$exitConflict = 6
$exitIo = 8
$exitIncompatible = 9
$exitLock = 10
# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

<#
    Strict-mode-safe property read. `Set-StrictMode -Version 2.0` throws on a property that does
    not exist, which is the wrong behaviour when reading a document written by another process.
#>
function Get-AxiomProp {
    param(
        [AllowNull()]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [AllowNull()]$Default = $null
    )
    if ($null -eq $Object) { return $Default }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $Default }
    return $property.Value
}

function Exit-Axiom {
    param([int]$Code, [string]$Message)
    if ($Message) { Write-AxiomDiag -Message $Message }
    exit $Code
}

<#
    Finish a run: optionally persist the envelope, then emit it (or its one-line summary) and
    exit with the code that matches the envelope.
#>
function Complete-AxiomRun {
    param(
        [Parameter(Mandatory = $true)]$Envelope,
        [Parameter(Mandatory = $true)][int]$Code,
        [Parameter(Mandatory = $true)][string]$Outcome,
        [Parameter(Mandatory = $true)][string]$Status,
        [Parameter(Mandatory = $true)][string]$Message,
        [bool]$Retryable = $false
    )
    [void](Complete-AxiomEnvelope -Envelope $Envelope -ExitCode $Code -Outcome $Outcome -Status $Status `
            -Message $Message -Retryable $Retryable -JsonOutPath $Out -EmitJson:([bool]$Json))
    exit $Code
}

<#
    Project a release-set artifact onto the install-result artifact record. The release set
    carries packaging detail (file_name, artifact_kind, install_relative_path) that the envelope
    schema deliberately does not; the contract requires exactly name, version and sha256.
#>
function New-AxiomEnvelopeArtifact {
    param(
        $Artifact,
        [AllowNull()][string]$InstalledPath = $null
    )
    return [ordered]@{
        name           = [string](Get-AxiomProp -Object $Artifact -Name 'name' -Default '')
        component      = [string](Get-AxiomProp -Object $Artifact -Name 'component' -Default '')
        version        = [string](Get-AxiomProp -Object $Artifact -Name 'version' -Default '')
        sha256         = [string](Get-AxiomProp -Object $Artifact -Name 'sha256' -Default '')
        size_bytes     = [long](Get-AxiomProp -Object $Artifact -Name 'size_bytes' -Default 0)
        verified       = $true
        installed_path = $InstalledPath
        source         = [string](Get-AxiomProp -Object $Artifact -Name 'source' -Default 'release-set')
    }
}

# --- install layout ---------------------------------------------------------
function Get-AxiomCliDir { param([string]$InstallRoot) return (Join-Path $InstallRoot 'cli') }
function Get-AxiomJournalPath { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'journal.json') }
function Get-AxiomStatePath { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'state.json') }
function Get-AxiomLockPath { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'transaction.lock') }
function Get-AxiomStagingDir { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'staging') }
function Get-AxiomGenerationsDir { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'generations') }
function Get-AxiomManifestPath { param([string]$InstallRoot) return (Join-Path $InstallRoot 'install-manifest.json') }
function Get-AxiomBinDir { param([string]$InstallRoot) return (Join-Path $InstallRoot 'bin') }

<#
    Restore the state this transaction's pre-image recorded. Called only from the commit-phase
    catch block, so a partially applied transaction never survives.
#>
function Invoke-AxiomRollback {
    param(
        [Parameter(Mandatory = $true)]$PreImage,
        [Parameter(Mandatory = $true)][string]$InstallRoot,
        [Parameter(Mandatory = $true)]$Envelope
    )

    $entryPath = [string](Get-AxiomProp -Object $PreImage -Name 'entrypoint_path' -Default '')
    $entryExisted = [bool](Get-AxiomProp -Object $PreImage -Name 'entrypoint_existed' -Default $false)
    $entryBackup = [string](Get-AxiomProp -Object $PreImage -Name 'entrypoint_backup' -Default '')

    if ($entryExisted) {
        if (-not [string]::IsNullOrEmpty($entryBackup) -and (Test-Path -LiteralPath $entryBackup -PathType Leaf)) {
            if (Test-Path -LiteralPath $entryPath -PathType Leaf) {
                Remove-Item -LiteralPath $entryPath -Force -ErrorAction SilentlyContinue
            }
            Move-Item -LiteralPath $entryBackup -Destination $entryPath -Force
            $Envelope.mutated = $false
        }
    } elseif (Test-Path -LiteralPath $entryPath -PathType Leaf) {
        Remove-Item -LiteralPath $entryPath -Force -ErrorAction SilentlyContinue
    }

    # PATH: restore the exact previous value, or remove the value when it did not exist.
    $pathPresent = [bool](Get-AxiomProp -Object $PreImage -Name 'user_path_present' -Default $false)
    $pathValue = [string](Get-AxiomProp -Object $PreImage -Name 'user_path_value' -Default '')
    $pathKind = [string](Get-AxiomProp -Object $PreImage -Name 'user_path_kind' -Default 'ExpandString')
    try {
        if ($pathPresent) {
            Set-AxiomUserPathValue -Value $pathValue -Kind $pathKind
        } elseif (Test-Path -LiteralPath $script:AxiomUserRegistryKey) {
            Remove-ItemProperty -LiteralPath $script:AxiomUserRegistryKey -Name $script:AxiomUserPathValueName -ErrorAction SilentlyContinue
        }
    } catch { }

    # A generation this transaction created is removed; a previous one is left alone.
    $generationCreated = [bool](Get-AxiomProp -Object $PreImage -Name 'generation_created' -Default $false)
    $generationPath = [string](Get-AxiomProp -Object $PreImage -Name 'generation_path' -Default '')
    if ($generationCreated -and -not [string]::IsNullOrEmpty($generationPath) -and (Test-Path -LiteralPath $generationPath)) {
        Remove-Item -LiteralPath $generationPath -Recurse -Force -ErrorAction SilentlyContinue
    }

    # state.json / install-manifest.json: restore or remove per the pre-image.
    foreach ($record in @(
            @{ path = (Get-AxiomStatePath $InstallRoot); existed = 'state_existed'; backup = 'state_backup' }
            @{ path = (Get-AxiomManifestPath $InstallRoot); existed = 'manifest_existed'; backup = 'manifest_backup' }
        )) {
        $existed = [bool](Get-AxiomProp -Object $PreImage -Name $record.existed -Default $false)
        $backup = [string](Get-AxiomProp -Object $PreImage -Name $record.backup -Default '')
        if ($existed) {
            if (-not [string]::IsNullOrEmpty($backup) -and (Test-Path -LiteralPath $backup -PathType Leaf)) {
                Move-Item -LiteralPath $backup -Destination $record.path -Force
            }
        } elseif (Test-Path -LiteralPath $record.path -PathType Leaf) {
            Remove-Item -LiteralPath $record.path -Force -ErrorAction SilentlyContinue
        }
    }

    # Service registration that this transaction created is removed again.
    $serviceRegistered = [bool](Get-AxiomProp -Object $PreImage -Name 'service_registered' -Default $false)
    $serviceTaskName = [string](Get-AxiomProp -Object $PreImage -Name 'service_task_name' -Default '')
    if ($serviceRegistered -and -not [string]::IsNullOrEmpty($serviceTaskName)) {
        try {
            Unregister-ScheduledTask -TaskName $serviceTaskName -Confirm:$false -ErrorAction SilentlyContinue
            foreach ($entry in @($Envelope.removed)) {
                if ($entry.kind -eq 'service-registration') {
                    $Envelope.removed = @($Envelope.removed | Where-Object { $_.kind -ne 'service-registration' })
                }
            }
        } catch { }
    }

    # Staging never survives a rollback.
    $staging = Get-AxiomStagingDir $InstallRoot
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
    }
}
# ---------------------------------------------------------------------------
# 1. Resolve and hash the release set. Its sha256 is the approval digest.
# ---------------------------------------------------------------------------

$documentPath = Resolve-AxiomReleaseSetPath -ReleaseSet $ReleaseSet

$envelope = New-AxiomEnvelopeBase -Operation 'install'
$envelope.limitations = @(
    'this installer is the Windows x64 lane of the axiom-cli distribution; other delivery platforms are separate installers and are not exercised here'
)

if (-not (Test-Path -LiteralPath $documentPath -PathType Leaf)) {
    Complete-AxiomRun -Envelope $envelope -Code $exitNotFound -Outcome 'refused' -Status 'error' `
        -Message ("release set not found: {0} (build one with packaging/windows/Build-ReleaseSet.ps1)" -f $documentPath)
}

$documentPath = (Resolve-Path -LiteralPath $documentPath).Path
$releaseSetDir = Split-Path -Parent $documentPath
$planDigest = Get-AxiomSha256Hex -Path $documentPath
$envelope.plan_digest = $planDigest

$plan = Read-AxiomJsonFile -Path $documentPath
if ($null -eq $plan) {
    Add-AxiomRefusal -Envelope $envelope -Check 'release-set-readable' -Reason 'release-set.json is not readable JSON'
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'error' `
        -Message ("release set is not readable JSON: {0}" -f $documentPath)
}

$schemaVersion = [int](Get-AxiomProp -Object $plan -Name 'schema_version' -Default -1)
$documentKind = [string](Get-AxiomProp -Object $plan -Name 'document_kind' -Default '')
$platform = [string](Get-AxiomProp -Object $plan -Name 'platform' -Default '')
$releaseVersion = [string](Get-AxiomProp -Object $plan -Name 'release_version' -Default '')
$entrypointName = [string](Get-AxiomProp -Object $plan -Name 'entrypoint_executable' -Default 'axiom-cli.exe')

$shapeErrors = New-Object System.Collections.Generic.List[string]
if ($schemaVersion -ne 1) { $shapeErrors.Add(("schema_version must be 1, got {0}" -f $schemaVersion)) }
if ($documentKind -ne 'axiom-cli-release-set') { $shapeErrors.Add(("document_kind must be 'axiom-cli-release-set', got '{0}'" -f $documentKind)) }
if ($platform -ne $script:AxiomPlatformId) { $shapeErrors.Add(("platform must be '{0}', got '{1}'" -f $script:AxiomPlatformId, $platform)) }
if ($releaseVersion -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') {
    $shapeErrors.Add(("release_version '{0}' is not a semantic version" -f $releaseVersion))
}
if ($shapeErrors.Count -gt 0) {
    foreach ($shapeError in $shapeErrors) {
        Add-AxiomRefusal -Envelope $envelope -Check 'release-set-shape' -Artifact $documentPath -Reason $shapeError
    }
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'error' `
        -Message ("release set is not a '{0}' document for platform '{1}'" -f 'axiom-cli-release-set', $script:AxiomPlatformId)
}

$envelope.release_set = [ordered]@{
    path            = $documentPath
    sha256          = $planDigest
    release_version = $releaseVersion
}

foreach ($planLimitation in @(Get-AxiomProp -Object $plan -Name 'limitations' -Default @())) {
    $envelope.limitations = @($envelope.limitations) + @([string]$planLimitation)
}

# ---------------------------------------------------------------------------
# 2. Verify every artifact this release carries: size first, then sha256.
#    A mismatch refuses the whole transaction; nothing is copied on a mismatch.
# ---------------------------------------------------------------------------

$planArtifacts = @(Get-AxiomProp -Object $plan -Name 'artifacts' -Default @())
if ($planArtifacts.Count -eq 0) {
    Add-AxiomRefusal -Envelope $envelope -Check 'artifacts-present' -Artifact $documentPath `
        -Reason 'the release set declares no artifacts, so there is nothing to install'
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'error' `
        -Message 'release set declares no artifacts'
}

$verified = New-Object System.Collections.Generic.List[object]
$sizeMismatch = $null
$digestMismatch = $null
$missingArtifact = $null

foreach ($artifact in $planArtifacts) {
    $name = [string](Get-AxiomProp -Object $artifact -Name 'name' -Default '')
    $expectedSize = [long](Get-AxiomProp -Object $artifact -Name 'size_bytes' -Default -1)
    $expectedSha = ([string](Get-AxiomProp -Object $artifact -Name 'sha256' -Default '')).ToLowerInvariant()

    if ([string]::IsNullOrEmpty($name)) {
        $missingArtifact = $artifact
        break
    }

    $sourcePath = Join-Path $releaseSetDir $name
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        $missingArtifact = $artifact
        break
    }

    $actualSize = [long](Get-Item -LiteralPath $sourcePath).Length
    if ($actualSize -ne $expectedSize) {
        $sizeMismatch = [ordered]@{ artifact = $artifact; actual_size = $actualSize; expected_size = $expectedSize }
        break
    }

    $actualSha = Get-AxiomSha256Hex -Path $sourcePath
    if ($actualSha -ne $expectedSha) {
        $digestMismatch = [ordered]@{ artifact = $artifact; actual_sha = $actualSha; expected_sha = $expectedSha }
        break
    }

    $record = New-AxiomEnvelopeArtifact -Artifact $artifact
    $record.installed_path = $null
    $verified.Add([ordered]@{
        envelope        = $record
        source_path     = $sourcePath
        name            = $name
        install_relpath = [string](Get-AxiomProp -Object $artifact -Name 'install_relative_path' -Default $name)
    })
}

if ($missingArtifact) {
    $missingName = [string](Get-AxiomProp -Object $missingArtifact -Name 'name' -Default '<unnamed>')
    Add-AxiomRefusal -Envelope $envelope -Check 'artifact-present' -Artifact $missingName `
        -Reason ("artifact '{0}' is declared by the release set but is not present in {1}" -f $missingName, $releaseSetDir)
    Complete-AxiomRun -Envelope $envelope -Code $exitNotFound -Outcome 'refused' -Status 'error' `
        -Message ("release-set artifact not found: {0}" -f $missingName)
}

if ($sizeMismatch) {
    $artifact = $sizeMismatch.artifact
    $name = [string](Get-AxiomProp -Object $artifact -Name 'name' -Default '<unnamed>')
    Add-AxiomRefusal -Envelope $envelope -Check 'artifact-size' -Artifact $name `
        -Expected ([string]$sizeMismatch.expected_size) -Actual ([string]$sizeMismatch.actual_size) `
        -Reason ("artifact '{0}' is {1} bytes on disk but the release set declares {2}" -f `
            $name, $sizeMismatch.actual_size, $sizeMismatch.expected_size)
    Complete-AxiomRun -Envelope $envelope -Code $exitIncompatible -Outcome 'refused' -Status 'refused' `
        -Message ("artifact size mismatch: {0}" -f $name)
}

if ($digestMismatch) {
    $artifact = $digestMismatch.artifact
    $name = [string](Get-AxiomProp -Object $artifact -Name 'name' -Default '<unnamed>')
    Add-AxiomRefusal -Envelope $envelope -Check 'artifact-sha256' -Artifact $name `
        -Expected $digestMismatch.expected_sha -Actual $digestMismatch.actual_sha `
        -Reason ("artifact '{0}' has sha256 {1} but the release set declares {2}" -f `
            $name, $digestMismatch.actual_sha, $digestMismatch.expected_sha)
    Complete-AxiomRun -Envelope $envelope -Code $exitIncompatible -Outcome 'refused' -Status 'refused' `
        -Message ("artifact digest mismatch: {0}" -f $name)
}

$envelope.artifacts = @($verified | ForEach-Object { $_.envelope })

# Artifacts this release does not carry are declared unverified, never installed.
$declaredComponents = @(Get-AxiomProp -Object $plan -Name 'declared_components' -Default @())
foreach ($declared in $declaredComponents) {
    $state = [string](Get-AxiomProp -Object $declared -Name 'state' -Default 'unverified')
    if ($state -eq 'carried') { continue }
    $envelope.unverified_artifacts = @($envelope.unverified_artifacts) + @([ordered]@{
            name      = [string](Get-AxiomProp -Object $declared -Name 'component' -Default 'unknown')
            component = [string](Get-AxiomProp -Object $declared -Name 'component' -Default 'unknown')
            reason    = [string](Get-AxiomProp -Object $declared -Name 'reason' -Default 'this release set carries no artifact for this component')
        })
}

# One row per installed component, naming its version and artifact digest.
$componentRows = New-Object System.Collections.Generic.List[object]
foreach ($item in $verified) {
    $record = $item.envelope
    $already = $null
    foreach ($row in $componentRows) {
        if ($row.component -eq $record.component) { $already = $row }
    }
    if ($null -eq $already) {
        $componentRows.Add([ordered]@{
                component         = $record.component
                installed_version = $releaseVersion
                artifact_sha256   = $record.sha256
                version_source    = 'release-set.json (sha256 ' + $planDigest + ')'
            })
    } elseif ($already.artifact_sha256 -ne $record.sha256) {
        $already.artifact_sha256 = $null
    }
}
$envelope.components = @($componentRows.ToArray())

# ---------------------------------------------------------------------------
# 3. Plan only, unless -Apply was given.
# ---------------------------------------------------------------------------

if (-not $Apply) {
    Write-AxiomDiag -Message ("plan: release {0}, {1} artifact(s), platform {2}" -f $releaseVersion, $verified.Count, $platform)
    Write-AxiomDiag -Message ("approve with: -Apply -ApproveDigest {0}" -f $planDigest)
    $envelope.dry_run = $true
    $envelope.mutated = $false
    $plannedService = Get-AxiomProp -Object $plan -Name 'service' -Default $null
    $envelope.service_registration = [ordered]@{
        kind       = $(if ($null -ne $plannedService) { [string](Get-AxiomProp -Object $plannedService -Name 'kind' -Default 'none') } else { 'none' })
        task_name  = $(if ($null -ne $plannedService) { [string](Get-AxiomProp -Object $plannedService -Name 'task_name' -Default '') } else { $null })
        owner      = 'axiom-cli'
        registered = $false
        removed    = $false
        reason     = [string](Get-AxiomProp -Object $plan -Name 'service_reason' -Default 'the release set declares no managed service for this platform')
    }
    Complete-AxiomRun -Envelope $envelope -Code $exitOk -Outcome 'planned' -Status 'ok' `
        -Message ("planned: release {0} with {1} verified artifact(s); nothing was changed" -f $releaseVersion, $verified.Count)
}

# ---------------------------------------------------------------------------
# 4. Approval. The digest is the plan; a mutating run refuses without it.
# ---------------------------------------------------------------------------

if ([string]::IsNullOrEmpty($ApproveDigest)) {
    Add-AxiomRefusal -Envelope $envelope -Check 'approval-digest' -Artifact $documentPath `
        -Expected $planDigest -Reason 'a mutating install requires -ApproveDigest bound to the release-set sha256'
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'refused' `
        -Message 'missing -ApproveDigest: a mutating install must be approved'
}

$approved = $ApproveDigest.Trim().ToLowerInvariant()
$envelope.approved_digest = $approved
if ($approved -ne $planDigest) {
    Add-AxiomRefusal -Envelope $envelope -Check 'approval-digest' -Artifact $documentPath `
        -Expected $planDigest -Actual $approved -Reason 'the approval digest does not match the release-set sha256'
    Complete-AxiomRun -Envelope $envelope -Code $exitAuthorization -Outcome 'refused' -Status 'refused' `
        -Message ("approval digest mismatch: expected {0}, got {1}" -f $planDigest, $approved)
}
# ---------------------------------------------------------------------------
# 5. Install root, lock, interrupted-transaction recovery, idempotency.
# ---------------------------------------------------------------------------

<#
    Semantic-version ordering, prerelease lower than release, build metadata ignored. Returns 1
    when A is newer than B, 0 when equal and -1 when A is older.
#>
function Compare-AxiomReleaseVersion {
    param([string]$A, [string]$B)

    $strip = { param($v) return ($v -replace '\+.*$', '') }
    $split = { param($v)
        $s = & $strip $v
        $dash = $s.IndexOf('-')
        if ($dash -lt 0) { return @{ core = $s; pre = '' } }
        return @{ core = $s.Substring(0, $dash); pre = $s.Substring($dash + 1) }
    }
    $sa = & $split $A
    $sb = & $split $B

    $pa = @($sa.core -split '\.')
    $pb = @($sb.core -split '\.')
    for ($i = 0; $i -lt 3; $i++) {
        $na = 0; $nb = 0
        if ($i -lt $pa.Count) { $na = [int]$pa[$i] }
        if ($i -lt $pb.Count) { $nb = [int]$pb[$i] }
        if ($na -gt $nb) { return 1 }
        if ($na -lt $nb) { return -1 }
    }
    if ([string]::IsNullOrEmpty($sa.pre) -and [string]::IsNullOrEmpty($sb.pre)) { return 0 }
    if ([string]::IsNullOrEmpty($sa.pre)) { return 1 }
    if ([string]::IsNullOrEmpty($sb.pre)) { return -1 }
    return [Math]::Sign([string]::CompareOrdinal($sa.pre, $sb.pre))
}

$resolvedRoot = $InstallRoot
if ([string]::IsNullOrEmpty($resolvedRoot)) { $resolvedRoot = Get-AxiomDefaultInstallRoot }
if (-not [System.IO.Path]::IsPathRooted($resolvedRoot)) {
    Add-AxiomRefusal -Envelope $envelope -Check 'install-root' -Artifact $resolvedRoot `
        -Reason 'the install root must be an absolute path'
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'refused' `
        -Message ("install root must be absolute: {0}" -f $resolvedRoot)
}
$resolvedRoot = $resolvedRoot.TrimEnd('\')
$binDir = Get-AxiomBinDir $resolvedRoot
$entrypointPath = Join-Path $binDir $entrypointName

$envelope.install_root = $resolvedRoot
$envelope.bin_dir = $binDir
$transactionId = New-AxiomTransactionId
$envelope.transaction_id = $transactionId

$lockPath = Get-AxiomLockPath $resolvedRoot
$lockStream = Enter-AxiomTransactionLock -Path $lockPath -TransactionId $transactionId
if ($null -eq $lockStream) {
    $holder = Read-AxiomJsonFile -Path $lockPath
    $holderPid = Get-AxiomProp -Object $holder -Name 'pid' -Default $null
    Complete-AxiomRun -Envelope $envelope -Code $exitLock -Outcome 'refused' -Status 'refused' -Retryable $true `
        -Message ("another axiom-cli transaction holds the lock at {0} (pid {1}); retry when it finishes" -f $lockPath, $holderPid)
}

try {
    # --- interrupted transaction recovery ---------------------------------
    $journalPath = Get-AxiomJournalPath $resolvedRoot
    if (Test-Path -LiteralPath $journalPath -PathType Leaf) {
        $journal = Read-AxiomJsonFile -Path $journalPath
        $pre = Get-AxiomProp -Object $journal -Name 'pre_image' -Default $null
        if ($null -ne $pre) {
            Invoke-AxiomRollback -PreImage $pre -InstallRoot $resolvedRoot -Envelope $envelope
            $envelope.interrupted_install_recovered = $true
            Write-AxiomDiag -Message ("recovered an interrupted install from {0}" -f $journalPath)
        }
        Remove-Item -LiteralPath $journalPath -Force -ErrorAction SilentlyContinue
    }

    $state = Read-AxiomJsonFile -Path (Get-AxiomStatePath $resolvedRoot)
    $installedVersion = [string](Get-AxiomProp -Object $state -Name 'release_version' -Default '')
    $stateArtifacts = @(Get-AxiomProp -Object $state -Name 'artifacts' -Default @())

    $entrypointExists = Test-Path -LiteralPath $entrypointPath -PathType Leaf
    $entrypointUnowned = $false
    if ($entrypointExists) {
        $entrypointUnowned = $true
        if ($null -ne $state) {
            foreach ($stateArtifact in $stateArtifacts) {
                $recordedPath = [string](Get-AxiomProp -Object $stateArtifact -Name 'installed_path' -Default '')
                if (-not [string]::IsNullOrEmpty($recordedPath) -and $recordedPath -ieq $entrypointPath) {
                    $entrypointUnowned = $false
                }
            }
        }
    }

    # --- idempotency ------------------------------------------------------
    $isIdempotent = $false
    if ($entrypointExists -and -not $entrypointUnowned -and $installedVersion -eq $releaseVersion -and $stateArtifacts.Count -eq $verified.Count) {
        $allRecorded = $true
        foreach ($item in $verified) {
            $matched = $false
            foreach ($stateArtifact in $stateArtifacts) {
                if ([string](Get-AxiomProp -Object $stateArtifact -Name 'name' -Default '') -eq $item.name -and
                    [string](Get-AxiomProp -Object $stateArtifact -Name 'sha256' -Default '') -eq $item.envelope.sha256) {
                    $matched = $true
                }
            }
            if (-not $matched) { $allRecorded = $false }
        }
        $installedSha = Get-AxiomSha256Hex -Path $entrypointPath
        $expectedEntrySha = ''
        foreach ($item in $verified) {
            if ($item.name -eq $entrypointName) { $expectedEntrySha = $item.envelope.sha256 }
        }
        if ($allRecorded -and $expectedEntrySha -ne '' -and $installedSha -eq $expectedEntrySha) {
            $isIdempotent = $true
        }
    }

    $pathEntry = Get-AxiomPathEntryForRoot -InstallRoot $resolvedRoot -BinDir $binDir
    $pathBefore = Get-AxiomUserPathSnapshot
    $pathWasPresent = Test-AxiomPathContains -Value ([string]$pathBefore.value) -Entry $pathEntry

    if ($isIdempotent -and $pathWasPresent) {
        foreach ($envelopeArtifact in $envelope.artifacts) {
            $envelopeArtifact.installed_path = $entrypointPath
            foreach ($item in $verified) {
                if ($item.name -eq $envelopeArtifact.name -and $item.name -ne $entrypointName) {
                    $envelopeArtifact.installed_path = Join-Path (Join-Path (Get-AxiomGenerationsDir $resolvedRoot) $releaseVersion) $item.install_relpath
                }
            }
        }
        $envelope.path_rule.entry = $pathEntry
        $envelope.path_rule.applied = $true
        $envelope.path_rule.previous_present = $true
        $envelope.path_rule.scope = 'user'
        $envelope.preserved = @($envelope.preserved) + @([ordered]@{
                path   = $resolvedRoot
                reason = 'an idempotent re-run changes nothing: the recorded generation is kept in place'
            })
        Complete-AxiomRun -Envelope $envelope -Code $exitOk -Outcome 'already_installed' -Status 'ok' `
            -Message ("release {0} is already installed at {1} with matching digests; nothing changed" -f $releaseVersion, $resolvedRoot)
    }

    # --- conflicts and incompatible transitions ---------------------------
    if ($entrypointUnowned -and -not $Force) {
        Add-AxiomRefusal -Envelope $envelope -Check 'entrypoint-unowned' -Artifact $entrypointName `
            -Reason ("{0} already exists and is not owned by an axiom-cli install; pass -Force to replace it" -f $entrypointPath)
        Complete-AxiomRun -Envelope $envelope -Code $exitConflict -Outcome 'refused' -Status 'refused' `
            -Message ("refusing to overwrite an unowned file: {0}" -f $entrypointPath)
    }

    if (-not [string]::IsNullOrEmpty($installedVersion) -and -not $AllowDowngrade) {
        if ((Compare-AxiomReleaseVersion -A $releaseVersion -B $installedVersion) -lt 0) {
            Add-AxiomRefusal -Envelope $envelope -Check 'no-downgrade' -Artifact $documentPath `
                -Expected $installedVersion -Actual $releaseVersion `
                -Reason ("release {0} is older than the installed {1}; pass -AllowDowngrade to proceed" -f $releaseVersion, $installedVersion)
            Complete-AxiomRun -Envelope $envelope -Code $exitIncompatible -Outcome 'refused' -Status 'refused' `
                -Message ("refusing a downgrade from {0} to {1}" -f $installedVersion, $releaseVersion)
        }
    }

    # --- stage, then re-verify the staged bytes ---------------------------
    $stagingDir = Get-AxiomStagingDir $resolvedRoot
    if (Test-Path -LiteralPath $stagingDir) { Remove-Item -LiteralPath $stagingDir -Recurse -Force }
    $stagingBin = Join-Path $stagingDir 'bin'
    New-Item -ItemType Directory -Force -Path $stagingBin | Out-Null

    foreach ($item in $verified) {
        $destination = Join-Path $stagingBin $item.name
        Copy-Item -LiteralPath $item.source_path -Destination $destination -Force
        $stagedSha = Get-AxiomSha256Hex -Path $destination
        if ($stagedSha -ne $item.envelope.sha256) {
            Add-AxiomRefusal -Envelope $envelope -Check 'staged-sha256' -Artifact $item.name `
                -Expected $item.envelope.sha256 -Actual $stagedSha `
                -Reason 'the staged copy does not match the verified source digest'
            Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue
            Complete-AxiomRun -Envelope $envelope -Code $exitIo -Outcome 'refused' -Status 'error' `
                -Message ("staged copy failed digest verification: {0}" -f $item.name)
        }
    }
    # -----------------------------------------------------------------------
    # 6. Commit. Journal the pre-image first, then mutate; roll back on any error.
    # -----------------------------------------------------------------------

    $machinePathBefore = Get-AxiomMachinePathSnapshot
    $generationDir = Join-Path (Get-AxiomGenerationsDir $resolvedRoot) $releaseVersion
    $generationCreated = -not (Test-Path -LiteralPath $generationDir)

    $expectedEntrypointSha = ''
    foreach ($item in $verified) {
        if ($item.name -eq $entrypointName) { $expectedEntrypointSha = $item.envelope.sha256 }
    }
    if ([string]::IsNullOrEmpty($expectedEntrypointSha)) {
        Add-AxiomRefusal -Envelope $envelope -Check 'entrypoint-declared' -Artifact $entrypointName `
            -Reason ("the release set does not declare the entrypoint artifact '{0}'" -f $entrypointName)
        Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'refused' `
            -Message ("release set declares no '{0}' artifact" -f $entrypointName)
    }

    $cliDir = Get-AxiomCliDir $resolvedRoot
    $rollbackDir = Join-Path $cliDir 'rollback'
    $journalPath = Get-AxiomJournalPath $resolvedRoot
    $statePath = Get-AxiomStatePath $resolvedRoot
    $manifestPath = Get-AxiomManifestPath $resolvedRoot

    $preImage = [ordered]@{
        entrypoint_path    = $entrypointPath
        entrypoint_existed = $entrypointExists
        entrypoint_backup  = (Join-Path $rollbackDir 'entrypoint.prev.exe')
        state_path         = $statePath
        state_existed      = (Test-Path -LiteralPath $statePath -PathType Leaf)
        state_backup       = (Join-Path $rollbackDir 'state.prev.json')
        manifest_path      = $manifestPath
        manifest_existed   = (Test-Path -LiteralPath $manifestPath -PathType Leaf)
        manifest_backup    = (Join-Path $rollbackDir 'install-manifest.prev.json')
        user_path_present  = [bool]$pathBefore.present
        user_path_value    = [string]$pathBefore.value
        user_path_kind     = [string]$pathBefore.kind
        generation_path    = $generationDir
        generation_created = $generationCreated
        service_registered = $false
        service_task_name  = ''
    }

    $journal = [ordered]@{
        schema_version  = 1
        document_kind   = 'axiom-cli-install-journal'
        transaction_id  = $transactionId
        operation       = 'install'
        release_version = $releaseVersion
        plan_digest     = $planDigest
        started_at      = (Get-AxiomTimestamp)
        pre_image       = $preImage
    }
    Write-AxiomJsonFile -Path $journalPath -Value $journal

    try {
        # Snapshot what we are about to replace, outside the mutation path.
        New-Item -ItemType Directory -Force -Path $rollbackDir | Out-Null
        if ($preImage.entrypoint_existed) {
            # Windows keeps a running image open. Rename the owned entrypoint out
            # of the active path before moving the staged replacement into it.
            # The running process retains its image while the rollback copy stays
            # under this transaction's owned directory.
            Move-Item -LiteralPath $entrypointPath -Destination $preImage.entrypoint_backup -Force
        }
        if ($preImage.state_existed) {
            Copy-Item -LiteralPath $statePath -Destination $preImage.state_backup -Force
        }
        if ($preImage.manifest_existed) {
            Copy-Item -LiteralPath $manifestPath -Destination $preImage.manifest_backup -Force
        }

        # Keep the previous generation: it is the rollback artifact the contract requires.
        New-Item -ItemType Directory -Force -Path $generationDir | Out-Null
        foreach ($item in $verified) {
            $generationDest = Join-Path $generationDir $item.install_relpath
            $generationParent = Split-Path -Parent $generationDest
            if ($generationParent -and -not (Test-Path -LiteralPath $generationParent)) {
                New-Item -ItemType Directory -Force -Path $generationParent | Out-Null
            }
            Copy-Item -LiteralPath $item.source_path -Destination $generationDest -Force
        }

        # Atomic swap of the entrypoint from staging. Same volume, so the move is a rename.
        New-Item -ItemType Directory -Force -Path $binDir | Out-Null
        $stagedEntry = Join-Path $stagingBin $entrypointName
        Move-Item -LiteralPath $stagedEntry -Destination $entrypointPath -Force

        $landedSha = Get-AxiomSha256Hex -Path $entrypointPath
        if ($landedSha -ne $expectedEntrypointSha) {
            throw ("the installed entrypoint has sha256 {0} but {1} was verified" -f $landedSha, $expectedEntrypointSha)
        }

        # Non-entrypoint artifacts are addressed from the generation, which is the installed
        # generation; the entrypoint is the only artifact on the per-user PATH.
        foreach ($item in $verified) {
            if ($item.name -eq $entrypointName) { continue }
            $generationInstalled = Join-Path $generationDir $item.install_relpath
            if ((Get-AxiomSha256Hex -Path $generationInstalled) -ne $item.envelope.sha256) {
                throw ("installed artifact digest mismatch: {0}" -f $item.name)
            }
        }
        # --- managed service registration (only when the release set declares one) ---
        $service = Get-AxiomProp -Object $plan -Name 'service' -Default $null
        $serviceReason = [string](Get-AxiomProp -Object $plan -Name 'service_reason' `
                -Default 'the release set declares no managed service for this platform')
        $serviceRegistered = $false
        $serviceTaskName = ''
        $serviceKindName = 'none'

        if ($null -ne $service) {
            $declaredTaskName = [string](Get-AxiomProp -Object $service -Name 'task_name' -Default '')
            $declaredKind = [string](Get-AxiomProp -Object $service -Name 'kind' -Default 'none')
            if (-not [string]::IsNullOrEmpty($declaredTaskName) -and $declaredKind -eq 'scheduled-task-at-logon') {
                $serviceArgv = @(Get-AxiomProp -Object $service -Name 'argv' -Default @())
                $account = ("{0}\{1}" -f $env:USERDOMAIN, $env:USERNAME)
                $action = New-ScheduledTaskAction -Execute $entrypointPath -Argument ($serviceArgv -join ' ')
                $principal = New-ScheduledTaskPrincipal -UserId $account -LogonType Interactive -RunLevel Limited
                $trigger = New-ScheduledTaskTrigger -AtLogOn -User $account
                $taskSettings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
                Register-ScheduledTask -TaskName $declaredTaskName -Action $action -Principal $principal `
                    -Trigger $trigger -Settings $taskSettings -Force | Out-Null
                $serviceRegistered = $true
                $serviceTaskName = $declaredTaskName
                $serviceKindName = 'scheduled-task-at-logon'
                # Update the journal so an interrupted run after this point removes the task.
                $preImage.service_registered = $true
                $preImage.service_task_name = $declaredTaskName
                Write-AxiomJsonFile -Path $journalPath -Value $journal
            }
        }

        # --- installed artifact rows and state --------------------------------
        $installedRows = New-Object System.Collections.Generic.List[object]
        foreach ($envelopeArtifact in $envelope.artifacts) {
            $artifactInstalledPath = $entrypointPath
            foreach ($item in $verified) {
                if ($item.name -eq $envelopeArtifact.name -and $item.name -ne $entrypointName) {
                    $artifactInstalledPath = Join-Path $generationDir $item.install_relpath
                }
            }
            $envelopeArtifact.installed_path = $artifactInstalledPath
            $installedRows.Add([ordered]@{
                    name           = $envelopeArtifact.name
                    component      = $envelopeArtifact.component
                    version        = $envelopeArtifact.version
                    sha256         = $envelopeArtifact.sha256
                    size_bytes     = [long]$envelopeArtifact.size_bytes
                    installed_path = $artifactInstalledPath
                    source         = $envelopeArtifact.source
                })
        }

        $stateService = [ordered]@{ kind = $serviceKindName; task_name = $null }
        if ($serviceRegistered) { $stateService.task_name = $serviceTaskName }
        Write-AxiomJsonFile -Path $statePath -Value ([ordered]@{
                schema_version  = 1
                document_kind   = 'axiom-cli-install-state'
                platform        = $script:AxiomPlatformId
                release_version = $releaseVersion
                plan_digest     = $planDigest
                transaction_id  = $transactionId
                installed_at    = (Get-AxiomTimestamp)
                install_root    = $resolvedRoot
                bin_dir         = $binDir
                entrypoint      = $entrypointPath
                generation      = $generationDir
                path_entry      = $pathEntry
                artifacts       = @($installedRows.ToArray())
                service         = $stateService
            })

        # --- shared per-user install manifest (later install steps append to it) ---
        $existingManifest = Read-AxiomJsonFile -Path $manifestPath
        $existingComponents = @(Get-AxiomProp -Object $existingManifest -Name 'components' -Default @())
        $mergedComponents = New-Object System.Collections.Generic.List[object]
        foreach ($existing in $existingComponents) {
            $existingName = [string](Get-AxiomProp -Object $existing -Name 'component' -Default '')
            $superseded = $false
            foreach ($row in $componentRows) {
                if ($row.component -eq $existingName) { $superseded = $true }
            }
            if (-not $superseded) { $mergedComponents.Add($existing) }
        }
        foreach ($row in $componentRows) {
            $componentArtifacts = New-Object System.Collections.Generic.List[object]
            foreach ($installedRow in $installedRows) {
                if ($installedRow.component -eq $row.component) {
                    $componentArtifacts.Add([ordered]@{
                            name           = $installedRow.name
                            sha256         = $installedRow.sha256
                            size_bytes     = $installedRow.size_bytes
                            installed_path = $installedRow.installed_path
                        })
                }
            }
            $mergedComponents.Add([ordered]@{
                    component         = $row.component
                    installed_version = $row.installed_version
                    version_source    = $row.version_source
                    artifacts         = @($componentArtifacts.ToArray())
                    installed_at      = (Get-AxiomTimestamp)
                    install_root      = $resolvedRoot
                })
        }
        Write-AxiomJsonFile -Path $manifestPath -Value ([ordered]@{
                schema_version  = 1
                document_kind   = 'axiom-cli-install-manifest'
                platform        = $script:AxiomPlatformId
                host            = (Get-AxiomHostInfo)
                updated_at      = (Get-AxiomTimestamp)
                release_version = $releaseVersion
                plan_digest     = $planDigest
                components      = @($mergedComponents.ToArray())
                preserved_roots = @('workspace state and the per-project graph output root (<project>\.axiom\graph) live outside this install root and are never written or removed by the distribution')
            })

        # --- per-user PATH rule (user scope only) -----------------------------
        if (-not $pathWasPresent) {
            $newPathValue = Add-AxiomPathEntry -Value ([string]$pathBefore.value) -Entry $pathEntry
            Set-AxiomUserPathValue -Value $newPathValue -Kind ([string]$pathBefore.kind)
        }
        $machinePathAfter = Get-AxiomMachinePathSnapshot
        if ($machinePathBefore -ne $machinePathAfter) {
            throw 'the machine-wide PATH changed during this transaction'
        }

        # --- post-commit health check ----------------------------------------
        $health = Get-AxiomProp -Object $plan -Name 'health_check' -Default $null
        $healthArgv = @('--help')
        $healthExpected = 0
        if ($null -ne $health) {
            $declaredHealthArgv = @(Get-AxiomProp -Object $health -Name 'argv' -Default @())
            if ($declaredHealthArgv.Count -gt 0) { $healthArgv = $declaredHealthArgv }
            $healthExpected = [int](Get-AxiomProp -Object $health -Name 'expected_exit_code' -Default 0)
        }
        $healthOutput = @(& $entrypointPath @healthArgv 2>&1 | ForEach-Object { [string]$_ })
        $healthCode = $LASTEXITCODE
        if ($healthCode -ne $healthExpected) {
            throw ("health check '{0} {1}' exited {2}, expected {3}: {4}" -f `
                $entrypointName, ($healthArgv -join ' '), $healthCode, $healthExpected, ($healthOutput -join ' | '))
        }
        # --- tidy: once durable, the journal and the in-transaction backups are done ---
        Remove-Item -LiteralPath $journalPath -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $rollbackDir) {
            Remove-Item -LiteralPath $rollbackDir -Recurse -Force -ErrorAction SilentlyContinue
        }
        $stagingPath = $stagingDir
        if (Test-Path -LiteralPath $stagingPath) {
            Remove-Item -LiteralPath $stagingPath -Recurse -Force
        }

        # --- report -----------------------------------------------------------
        $serviceTaskNameOut = $null
        if ($serviceRegistered) { $serviceTaskNameOut = $serviceTaskName }

        $envelope.dry_run = $false
        $envelope.mutated = $true
        $envelope.path_rule.scope = 'user'
        $envelope.path_rule.entry = $pathEntry
        $envelope.path_rule.applied = $true
        $envelope.path_rule.previous_present = $pathWasPresent
        $envelope.path_rule.machine_wide_change = $false
        $envelope.service_registration = [ordered]@{
            kind       = $serviceKindName
            task_name  = $serviceTaskNameOut
            owner      = 'axiom-cli'
            registered = $serviceRegistered
            removed    = $false
            reason     = $serviceReason
        }
        $envelope.preserved = @(
            [ordered]@{
                path   = $resolvedRoot
                reason = 'the per-user install root holds user data outside the distributed binaries and is never deleted or rewritten by an install'
            },
            [ordered]@{
                path   = $generationDir
                reason = 'the installed generation is retained as the rollback artifact the contract requires'
            }
        )
        $envelope.removed = @(
            [ordered]@{ path = $stagingPath; kind = 'staging' }
        )
        $envelope.limitations = @($envelope.limitations) + @(
            ("the installed entrypoint answered '{0} {1}' with exit {2} after commit" -f $entrypointName, ($healthArgv -join ' '), $healthCode),
            'the per-user PATH entry is written to HKCU\Environment; a shell that was already running keeps its old environment until it is restarted'
        )

        $outcome = 'installed'
        if ($envelope.interrupted_install_recovered) { $outcome = 'recovered_and_installed' }
        $summary = ("installed release {0} to {1}: {2} digest-verified artifact(s), user PATH entry '{3}', elevation_required=false" -f `
                $releaseVersion, $resolvedRoot, $verified.Count, $pathEntry)
        Complete-AxiomRun -Envelope $envelope -Code $exitOk -Outcome $outcome -Status 'ok' -Message $summary
    } catch {
        $failureReason = [string]$_.Exception.Message
        Add-AxiomRefusal -Envelope $envelope -Check 'commit' -Artifact $documentPath -Reason $failureReason
        try {
            Invoke-AxiomRollback -PreImage $preImage -InstallRoot $resolvedRoot -Envelope $envelope
        } catch {
            $envelope.limitations = @($envelope.limitations) + @(
                ("rollback reported: {0}" -f [string]$_.Exception.Message)
            )
        }
        Remove-Item -LiteralPath $journalPath -Force -ErrorAction SilentlyContinue
        Complete-AxiomRun -Envelope $envelope -Code $exitIo -Outcome 'rolled_back' -Status 'error' -Retryable $true `
            -Message ("install transaction rolled back: {0}" -f $failureReason)
    }
} finally {
    Exit-AxiomTransactionLock -Stream $lockStream -Path $lockPath
}
