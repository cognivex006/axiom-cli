#requires -version 5.1
<#
.SYNOPSIS
    Assemble a Windows x64 axiom-cli release set and its digest manifest.

.DESCRIPTION
    Owner: axiom-cli. Task J-004.

    A release set is the unit the Windows installer consumes: a directory holding the
    distributed entrypoint `axiom-cli.exe`, any component artifacts this release
    actually carries, and `release-set.json` - a manifest that records the version,
    the byte length and the sha256 of every artifact, the honest state of the pinned
    ecosystem components this release does not carry, and the health check the
    installer must run after it commits.

    The manifest is the plan. Its own sha256 is the approval digest that
    Install-AxiomCli.ps1 requires, so this script prints it:

        $set = & .\packaging\windows\Build-ReleaseSet.ps1 -OutDir .\out\win-x64 -Build
        & .\installers\windows\Install-AxiomCli.ps1 -ReleaseSet $set.ReleaseSetPath -Apply `
            -ApproveDigest $set.PlanDigest

    This script never invents a version it cannot read, never claims an artifact it
    cannot hash, and never writes outside -OutDir.

.PARAMETER OutDir
    Directory the release set is assembled into. Must not already exist, so a
    packaging run cannot replace unrelated or human-owned content.

.PARAMETER CliExe
    A prebuilt `axiom-cli.exe`. Defaults to `<repo>\target\release\axiom-cli.exe`.

.PARAMETER Build
    Run `cargo build --release --locked` before assembling when the default binary is
    absent or stale.

.PARAMETER ComponentArtifact
    Additional real artifact to carry, as `<component>=<path>`. Repeatable. The file
    name becomes the artifact name. Component artifacts whose owning repository has
    not published a release are simply not passed here, and the manifest records them
    as unverified rather than inventing them.

.PARAMETER CoreManifest
    Path to `axiom-graphd\release\core-manifest.json`. When supplied, the release set
    records the core release version and the honest archive state that manifest
    declares. When omitted, the declaration records that no core manifest was read.

.PARAMETER CoreCandidateManifest
    Exact K-302 unsigned native candidate manifest. Its binary/archive digests
    must match artifacts carried in this set; the set remains unpublished.

.PARAMETER ServiceTaskName
    Declare a per-user scheduled-task-at-logon registration for this release. The
    caller owns the semantics; this packaging layer only records what is declared.
    Omitted means the release declares no service, which is the honest default while
    the graph daemon release is not built.

.PARAMETER ServiceArgv
    Argument vector for the declared service task.

.PARAMETER HealthArgv
    Argument vector the installer runs against the installed entrypoint after commit.
    Defaults to `--help`, which every axiom-cli build answers with exit 0.

.PARAMETER HealthExpectedExitCode
    Exit code the health check must observe. Defaults to 0.

.PARAMETER Json
    Emit exactly one JSON object on stdout: the release-set path, its sha256 and the
    assembled version. Diagnostics go to stderr.

.EXAMPLE
    pwsh -File .\packaging\windows\Build-ReleaseSet.ps1 -OutDir .\out\win-x64 -Build -Json

.NOTES
    Exit codes follow the canonical CLI vocabulary: 0 success, 2 validation,
    3 not found, 8 I/O, 9 incompatible.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$CliExe,
    [switch]$Build,
    [string[]]$ComponentArtifact = @(),
    [string]$CoreManifest,
    [string]$CoreCandidateManifest,
    [string]$Version,
    [string]$ServiceTaskName,
    [string[]]$ServiceArgv = @(),
    [string[]]$HealthArgv = @('--help'),
    [int]$HealthExpectedExitCode = 0,
    [switch]$Json
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$common = Join-Path (Split-Path -Parent $PSScriptRoot) '..\installers\windows\AxiomCli.Windows.Common.ps1'
. (Resolve-Path -LiteralPath $common).Path

$exitValidation = 2
$exitNotFound = 3
$exitIo = 8
$exitIncompatible = 9

function Exit-Axiom {
    param([int]$Code, [string]$Message)
    if ($Message) { Write-AxiomDiag -Message $Message }
    exit $Code
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path

# --- version -----------------------------------------------------------------
$resolvedVersion = $Version
if ([string]::IsNullOrEmpty($resolvedVersion)) {
    $versionFile = Join-Path $repoRoot 'VERSION'
    if (-not (Test-Path -LiteralPath $versionFile)) {
        Exit-Axiom -Code $exitNotFound -Message "no -Version supplied and $versionFile does not exist"
    }
    $resolvedVersion = ([System.IO.File]::ReadAllText($versionFile)).Trim()
}
if ($resolvedVersion -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') {
    Exit-Axiom -Code $exitValidation -Message "-Version '$resolvedVersion' is not a semantic version"
}

# --- entrypoint binary -------------------------------------------------------
$cliSource = $CliExe
if ([string]::IsNullOrEmpty($cliSource)) {
    $cliSource = Join-Path $repoRoot 'target\release\axiom-cli.exe'
    if ((-not (Test-Path -LiteralPath $cliSource)) -or $Build) {
        if ($Build) {
            Write-AxiomDiag -Message "building axiom-cli (cargo build --release --locked)"
            Push-Location $repoRoot
            try {
                & cargo build --release --locked 2>&1 | ForEach-Object { Write-AxiomDiag -Message ([string]$_) }
                if ($LASTEXITCODE -ne 0) {
                    Exit-Axiom -Code $exitIo -Message "cargo build --release --locked exited $LASTEXITCODE"
                }
            } finally {
                Pop-Location
            }
        }
    }
}
if (-not (Test-Path -LiteralPath $cliSource)) {
    Exit-Axiom -Code $exitNotFound -Message ("entrypoint binary not found at {0}; build it or pass -CliExe" -f $cliSource)
}
$cliSource = (Resolve-Path -LiteralPath $cliSource).Path

# --- output directory --------------------------------------------------------
if (Test-Path -LiteralPath $OutDir) {
    Exit-Axiom -Code $exitValidation -Message "-OutDir already exists; choose a new release-set directory"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outRoot = (Resolve-Path -LiteralPath $OutDir).Path

$artifacts = New-Object System.Collections.Generic.List[object]

function Add-AxiomReleaseArtifact {
    param(
        [string]$Name,
        [string]$Component,
        [string]$Version,
        [string]$Kind,
        [string]$Source,
        [string]$InstallRelativePath
    )
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        Exit-Axiom -Code $exitNotFound -Message ("artifact source not found: {0}" -f $Source)
    }
    $target = Join-Path $outRoot $Name
    Copy-Item -LiteralPath $Source -Destination $target -Force
    $item = Get-Item -LiteralPath $target
    $artifacts.Add([ordered]@{
        name                  = $Name
        file_name             = $Name
        component             = $Component
        version               = $Version
        artifact_kind         = $Kind
        install_relative_path = $InstallRelativePath
        source                = 'local-build'
        size_bytes            = [long]$item.Length
        sha256                = (Get-AxiomSha256Hex -Path $target)
    })
}

Add-AxiomReleaseArtifact -Name 'axiom-cli.exe' -Component 'axiom-cli' -Version $resolvedVersion `
    -Kind 'executable' -Source $cliSource -InstallRelativePath 'axiom-cli.exe'

foreach ($declaration in $ComponentArtifact) {
    $split = $declaration.IndexOf('=')
    if ($split -lt 1 -or $split -ge ($declaration.Length - 1)) {
        Exit-Axiom -Code $exitValidation -Message "-ComponentArtifact must be '<component>=<path>', got '$declaration'"
    }
    $component = $declaration.Substring(0, $split).Trim()
    $path = $declaration.Substring($split + 1).Trim()
    $name = Split-Path -Leaf $path
    $kind = 'resources'
    if ($name -match '\.exe$') { $kind = 'executable' }
    Add-AxiomReleaseArtifact -Name $name -Component $component -Version $resolvedVersion `
        -Kind $kind -Source $path -InstallRelativePath $name
}

# --- declared component set --------------------------------------------------
$coreNote = 'no axiom-graphd core manifest was read at assembly time, so this release set makes no claim about the core release state'
$coreVersion = $null
$coreArchiveState = 'unknown'
$coreVersionSource = 'axiom-graphd/release/core-manifest.json'
if ($CoreManifest -and $CoreCandidateManifest) {
    Exit-Axiom -Code $exitValidation -Message '-CoreManifest and -CoreCandidateManifest are mutually exclusive'
}
if (-not [string]::IsNullOrEmpty($CoreManifest)) {
    if (-not (Test-Path -LiteralPath $CoreManifest -PathType Leaf)) {
        Exit-Axiom -Code $exitNotFound -Message "-CoreManifest not found: $CoreManifest"
    }
    $core = Read-AxiomJsonFile -Path $CoreManifest
    if ($null -eq $core) {
        Exit-Axiom -Code $exitValidation -Message "-CoreManifest is not readable JSON: $CoreManifest"
    }
    $coreVersion = [string]$core.release.version
    $win = $null
    foreach ($target in @($core.release.targets)) {
        if ([string]$target.platform -eq 'windows-x64') { $win = $target }
    }
    if ($win) { $coreArchiveState = [string]$win.archive.state }
    $coreNote = ('read from {0}: release.version={1}, windows-x64 archive.state={2}, revision={3}' -f `
        $CoreManifest, $coreVersion, $coreArchiveState, [string]$core.release.revision)
}
if (-not [string]::IsNullOrEmpty($CoreCandidateManifest)) {
    if (-not (Test-Path -LiteralPath $CoreCandidateManifest -PathType Leaf)) {
        Exit-Axiom -Code $exitNotFound -Message 'core candidate manifest is missing'
    }
    $candidate = Read-AxiomJsonFile -Path $CoreCandidateManifest
    if ($null -eq $candidate -or [string]$candidate.platform -cne 'windows-x64' -or
        [string]$candidate.version -cne $resolvedVersion -or
        [string]$candidate.signing -cne 'unsigned' -or
        [string]$candidate.publication -cne 'not_published' -or
        [string]$candidate.source_revision -cnotmatch '^[0-9a-f]{40}$') {
        Exit-Axiom -Code $exitIncompatible -Message 'core candidate identity or provenance is invalid'
    }
    $expectedCore = @('axiom.exe', 'axiom-graphd.exe')
    foreach ($name in $expectedCore) {
        $record = @($candidate.binaries | Where-Object { [string]$_.name -ceq $name })
        $carried = @($artifacts | Where-Object { [string]$_.name -ceq $name })
        if ($record.Count -ne 1 -or $carried.Count -ne 1 -or
            [string]$record[0].sha256 -cne [string]$carried[0].sha256 -or
            [long]$record[0].size_bytes -ne [long]$carried[0].size_bytes) {
            Exit-Axiom -Code $exitIncompatible -Message ("core candidate binary does not match carried artifact: {0}" -f $name)
        }
    }
    $coreArchiveName = [string]$candidate.archive.name
    $coreArchive = @($artifacts | Where-Object { [string]$_.name -ceq $coreArchiveName })
    if ($coreArchive.Count -ne 1 -or [string]$coreArchive[0].sha256 -cne [string]$candidate.archive.sha256) {
        Exit-Axiom -Code $exitIncompatible -Message 'core candidate archive does not match carried artifact'
    }
    $coreVersion = [string]$candidate.version
    $coreArchiveState = 'candidate_verified'
    $coreVersionSource = 'axiom-graphd/evidence/K-302/candidate-manifest.json'
    $coreNote = ('unsigned Windows x64 core candidate at immutable revision {0}; archive and both native binaries are digest-verified' -f [string]$candidate.source_revision)
}

$declared = New-Object System.Collections.Generic.List[object]
$declared.Add([ordered]@{
    component             = 'axiom-cli'
    role                  = 'distribution-entrypoint'
    installed_version     = $resolvedVersion
    version_source        = 'axiom-cli/VERSION'
    version_source_status = 'declared'
    artifacts             = @('axiom-cli.exe')
    state                 = 'carried'
    reason                = 'this release set carries and digest-verifies the entrypoint'
})
foreach ($component in @(
        @{ name = 'axiom-graphd'; role = 'daemon'; artifacts = @('axiom-graphd.exe') },
        @{ name = 'axiom'; role = 'engine-cli'; artifacts = @('axiom.exe') })) {
    $carried = @()
    foreach ($artifact in $artifacts) {
        if ($artifact.component -eq $component.name) { $carried += $artifact.name }
    }
    $state = 'unverified'
    $reason = ('{0}: no artifact for this component is carried by the release set ({1})' -f $component.name, $coreNote)
    if ($carried.Count -gt 0) {
        $state = 'carried'
        $reason = ('{0}: artifact(s) {1} are carried and digest-verified' -f $component.name, ($carried -join ', '))
    }
    $declared.Add([ordered]@{
        component             = $component.name
        role                  = $component.role
        installed_version     = $coreVersion
        version_source        = $coreVersionSource
        version_source_status = $coreArchiveState
        artifacts             = $component.artifacts
        state                 = $state
        reason                = $reason
    })
}
foreach ($component in @(
        @{ name = 'axiom-mcp'; role = 'gateway'; source = 'axiom-mcp package metadata' },
        @{ name = 'skills'; role = 'bundle'; source = 'axiom-skills release manifest' })) {
    $carried = @()
    foreach ($artifact in $artifacts) {
        if ($artifact.component -eq $component.name) { $carried += $artifact.name }
    }
    $state = 'unverified'
    $reason = ('{0}: this release set declares no version source it can verify and carries no artifact for it' -f $component.name)
    if ($carried.Count -gt 0) {
        $state = 'carried'
        $reason = ('{0}: artifact(s) {1} are carried and digest-verified' -f $component.name, ($carried -join ', '))
    }
    $declared.Add([ordered]@{
        component             = $component.name
        role                  = $component.role
        installed_version     = $null
        version_source        = $component.source
        version_source_status = 'undeclared-at-assembly'
        artifacts             = @()
        state                 = $state
        reason                = $reason
    })
}

# --- service declaration -----------------------------------------------------
$service = $null
$serviceReason = ('the Windows managed service named by the platform matrix is the scheduled task at logon that hosts the graph daemon; the daemon lifecycle is owned by axiom-graphd and this release ({0}) does not carry or build it, so this release set declares no service and registers none' -f $coreNote)
if (-not [string]::IsNullOrEmpty($ServiceTaskName)) {
    $service = [ordered]@{
        kind      = 'scheduled-task-at-logon'
        owner     = 'axiom-cli'
        task_name = $ServiceTaskName
        argv      = @($ServiceArgv)
        run_level = 'limited'
        note      = 'declared by the caller of Build-ReleaseSet.ps1; the installer registers and the uninstaller removes exactly this registration'
    }
    $serviceReason = ('the release set declares a per-user scheduled-task-at-logon registration named {0}' -f $ServiceTaskName)
}

# --- manifest ----------------------------------------------------------------
$limitations = New-Object System.Collections.Generic.List[string]
if ($coreArchiveState -notin @('built', 'candidate_verified')) {
    $limitations.Add('the pinned core release is not built (' + $coreNote + '), so axiom-graphd.exe and axiom.exe are not carried by this release set and are recorded as unverified rather than installed')
}
if ($coreArchiveState -eq 'candidate_verified') {
    $limitations.Add('the core archive and binaries are verified unsigned local candidates; no released or trusted core provenance is claimed')
}
$limitations.Add('this release set is assembled from a local build; it is not a published release and carries no signature. Publication, signing and the update channel are owned by separate tasks')
$limitations.Add('the installation engine, the daemon service lifecycle and the per-component update transaction stay owned by axiom-graphd; this release set wraps them and never re-implements them')

# PowerShell refuses to build an `[ordered]@{ k = @($genericListOfObject) }` literal
# ("Argument types do not match", reproduced on 5.1 and 7.6). Materialise plain arrays
# first; the canonical writer then emits them as JSON arrays.
$artifactArray = $artifacts.ToArray()
$declaredArray = $declared.ToArray()
$limitationArray = $limitations.ToArray()

$manifest = [ordered]@{
    schema_version           = 1
    spec_version             = $script:AxiomSpecVersion
    document_kind            = 'axiom-cli-release-set'
    platform                 = $script:AxiomPlatformId
    os                       = 'windows'
    arch                     = 'x86_64'
    release_version          = $resolvedVersion
    entrypoint_executable    = $script:AxiomEntrypoint
    distribution_repository  = 'https://github.com/cognivex006/axiom-cli'
    assembled_by             = 'packaging/windows/Build-ReleaseSet.ps1'
    assembled_at             = (Get-AxiomTimestamp)
    health_check             = [ordered]@{
        argv               = @($HealthArgv)
        expected_exit_code = $HealthExpectedExitCode
    }
    artifacts                = $artifactArray
    declared_components      = $declaredArray
    service                  = $service
    service_reason           = $serviceReason
    forbidden_prerequisites  = @('elevation', 'bash', 'wsl', 'docker', 'nodejs', 'compiler-at-install-time')
    limitations              = $limitationArray
}

$manifestPath = Join-Path $outRoot 'release-set.json'
Write-AxiomJsonFile -Path $manifestPath -Value $manifest
$planDigest = Get-AxiomSha256Hex -Path $manifestPath

if ($Json) {
    [Console]::Out.WriteLine((ConvertTo-AxiomJson -Value ([ordered]@{
                    outcome          = 'assembled'
                    release_set_path = $manifestPath
                    release_version  = $resolvedVersion
                    plan_digest      = $planDigest
                    artifact_count   = $artifacts.Count
                    artifacts        = @($artifactArray | ForEach-Object {
                            [ordered]@{ name = $_.name; version = $_.version; size_bytes = $_.size_bytes; sha256 = $_.sha256 }
                        })
                })))
} else {
    Write-AxiomDiag -Message ('release set: {0}' -f $manifestPath)
    Write-AxiomDiag -Message ('version:     {0}' -f $resolvedVersion)
    Write-AxiomDiag -Message ('approve:     -ApproveDigest {0}' -f $planDigest)
}
exit 0
