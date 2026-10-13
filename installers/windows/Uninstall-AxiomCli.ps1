#requires -version 5.1
<#
.SYNOPSIS
    Remove the axiom-cli Windows x64 distribution from a per-user location.

.DESCRIPTION
    Owner: axiom-cli.

    The counterpart of Install-AxiomCli.ps1. It removes only what the distribution owns: the
    installed entrypoint, the recorded generation, the recorded per-user PATH entry and the
    recorded managed-service registration. It preserves the per-user install root itself and
    everything the distribution never wrote - above all the per-project graph output root
    (<project>\.axiom\graph) and any workspace state.

    It never recursively deletes `.axiom`. Removing the install root as well requires the
    explicit `-PurgeData` option, and because that changes the plan it changes the plan digest:
    it is a separate approval, not a rider on the binary-removal approval.

    Approval is bound to an uninstall-plan document, whose canonical JSON is hashed to produce
    `plan_digest`:

        # plan only (mutates nothing)
        & .\installers\windows\Uninstall-AxiomCli.ps1 -Json
        # approved removal
        & .\installers\windows\Uninstall-AxiomCli.ps1 -Apply -ApproveDigest <sha256> -Json

    Exit codes follow the canonical CLI vocabulary:

        0  success (removed, or planned)
        2  validation (missing or malformed approval digest)
        3  not found (nothing is installed at this root)
        5  approval refused (plan digest mismatch)
        8  I/O or internal error
       10  lock unavailable

.PARAMETER InstallRoot
    Per-user install root. Defaults to `%LOCALAPPDATA%\Axiom`.

.PARAMETER Apply
    Commit the removal. Without it the run only plans and mutates nothing.

.PARAMETER ApproveDigest
    The sha256 of the uninstall plan, as printed by a planning run. Required with `-Apply`.

.PARAMETER PurgeData
    Also remove the per-user install root directory. Off by default; it is a separate decision
    and therefore a separate plan digest.

.PARAMETER Json
    Emit exactly one JSON object (the install-result envelope, operation `uninstall`) on stdout.

.PARAMETER Out
    Also write the envelope to this file.

.EXAMPLE
    pwsh -File .\installers\windows\Uninstall-AxiomCli.ps1 -Apply -ApproveDigest <sha256> -Json

.NOTES
    Diagnostics always go to stderr, so `-Json` leaves exactly one JSON object on stdout.
#>
[CmdletBinding()]
param(
    [string]$InstallRoot,
    [switch]$Apply,
    [string]$ApproveDigest,
    [switch]$PurgeData,
    [switch]$Json,
    [string]$Out
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

. (Join-Path $PSScriptRoot 'AxiomCli.Windows.Common.ps1')

$exitOk = 0
$exitValidation = 2
$exitNotFound = 3
$exitAuthorization = 5
$exitIo = 8
$exitLock = 10

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

function Get-AxiomCliDir { param([string]$InstallRoot) return (Join-Path $InstallRoot 'cli') }
function Get-AxiomJournalPath { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'journal.json') }
function Get-AxiomStatePath { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'state.json') }
function Get-AxiomLockPath { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'transaction.lock') }
function Get-AxiomStagingDir { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'staging') }
function Get-AxiomGenerationsDir { param([string]$InstallRoot) return (Join-Path (Get-AxiomCliDir $InstallRoot) 'generations') }
function Get-AxiomManifestPath { param([string]$InstallRoot) return (Join-Path $InstallRoot 'install-manifest.json') }
function Get-AxiomBinDir { param([string]$InstallRoot) return (Join-Path $InstallRoot 'bin') }

$envelope = New-AxiomEnvelopeBase -Operation 'uninstall'
$envelope.limitations = @(
    'this uninstaller is the Windows x64 lane of the axiom-cli distribution; other delivery platforms are separate installers'
)

# ---------------------------------------------------------------------------
# 1. Resolve the root and build the plan.
# ---------------------------------------------------------------------------

$resolvedRoot = $InstallRoot
if ([string]::IsNullOrEmpty($resolvedRoot)) { $resolvedRoot = Get-AxiomDefaultInstallRoot }
if (-not [System.IO.Path]::IsPathRooted($resolvedRoot)) {
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'refused' -Status 'refused' `
        -Message ("install root must be absolute: {0}" -f $resolvedRoot)
}
$resolvedRoot = $resolvedRoot.TrimEnd('\')

$binDir = Get-AxiomBinDir $resolvedRoot
$cliDir = Get-AxiomCliDir $resolvedRoot
$entrypointName = $script:AxiomEntrypoint
$entrypointPath = Join-Path $binDir $entrypointName

$envelope.install_root = $resolvedRoot
$envelope.bin_dir = $binDir

$statePath = Get-AxiomStatePath $resolvedRoot
$state = Read-AxiomJsonFile -Path $statePath

if ($null -eq $state) {
    $envelope.preserved = @([ordered]@{
            path   = $resolvedRoot
            reason = 'nothing is installed here, so nothing was removed and nothing was touched'
        })
    Complete-AxiomRun -Envelope $envelope -Code $exitNotFound -Outcome 'not_removed' -Status 'refused' `
        -Message ("no axiom-cli install state at {0}; nothing to remove" -f $statePath)
}

$installedVersion = [string](Get-AxiomProp -Object $state -Name 'release_version' -Default '')
$generationPath = [string](Get-AxiomProp -Object $state -Name 'generation' -Default '')
$recordedEntrypoint = [string](Get-AxiomProp -Object $state -Name 'entrypoint' -Default $entrypointPath)
$recordedPathEntry = [string](Get-AxiomProp -Object $state -Name 'path_entry' -Default '')
$stateArtifacts = @(Get-AxiomProp -Object $state -Name 'artifacts' -Default @())
$stateService = Get-AxiomProp -Object $state -Name 'service' -Default $null
$serviceKind = [string](Get-AxiomProp -Object $stateService -Name 'kind' -Default 'none')
$serviceTaskName = [string](Get-AxiomProp -Object $stateService -Name 'task_name' -Default '')

# A recorded entrypoint that does not match this root's layout is not ours to remove.
$entrypointIsOurs = $recordedEntrypoint -ieq $entrypointPath
$entrypointExists = Test-Path -LiteralPath $entrypointPath -PathType Leaf
$entrypointSha = ''
if ($entrypointExists) { $entrypointSha = Get-AxiomSha256Hex -Path $entrypointPath }

$ownedGeneration = ''
if (-not [string]::IsNullOrEmpty($generationPath) -and
    $generationPath.StartsWith($resolvedRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
    (Test-Path -LiteralPath $generationPath)) {
    $ownedGeneration = $generationPath
}

$purge = [bool]$PurgeData
$plan = [ordered]@{
    schema_version  = 1
    document_kind   = 'axiom-cli-uninstall-plan'
    platform        = $script:AxiomPlatformId
    install_root    = $resolvedRoot
    release_version = $installedVersion
    purge_data      = $purge
    targets         = [ordered]@{
        entrypoint     = [ordered]@{ path = $entrypointPath; present = $entrypointExists; sha256 = $entrypointSha; owned = $entrypointIsOurs }
        generations    = @($ownedGeneration)
        cli_dir        = $cliDir
        path_entry     = $recordedPathEntry
        manifest       = (Get-AxiomManifestPath $resolvedRoot)
        service        = [ordered]@{ kind = $serviceKind; task_name = $(if ($serviceTaskName) { $serviceTaskName } else { $null }) }
        install_root   = $resolvedRoot
    }
}
$planDigest = Get-AxiomSha256HexOfText -Text (ConvertTo-AxiomJsonText -Value $plan)
$envelope.plan_digest = $planDigest
$envelope.release_set = $null
$envelope.artifacts = @($stateArtifacts | ForEach-Object {
        [ordered]@{
            name           = [string](Get-AxiomProp -Object $_ -Name 'name' -Default '')
            component      = [string](Get-AxiomProp -Object $_ -Name 'component' -Default '')
            version        = [string](Get-AxiomProp -Object $_ -Name 'version' -Default '')
            sha256         = [string](Get-AxiomProp -Object $_ -Name 'sha256' -Default '')
            size_bytes     = [long](Get-AxiomProp -Object $_ -Name 'size_bytes' -Default 0)
            verified       = $true
            installed_path = (Get-AxiomProp -Object $_ -Name 'installed_path' -Default $null)
            source         = [string](Get-AxiomProp -Object $_ -Name 'source' -Default 'recorded-in-state')
        }
    })
$envelope.components = @($stateArtifacts | ForEach-Object {
        [ordered]@{
            component         = [string](Get-AxiomProp -Object $_ -Name 'component' -Default '')
            installed_version = $installedVersion
            artifact_sha256   = [string](Get-AxiomProp -Object $_ -Name 'sha256' -Default '')
            version_source    = 'axiom-cli-install-state (recorded by the install transaction)'
        }
    })

# ---------------------------------------------------------------------------
# 2. Plan only, unless -Apply.
# ---------------------------------------------------------------------------

if (-not $Apply) {
    Write-AxiomDiag -Message ("uninstall plan for release {0} at {1}" -f $installedVersion, $resolvedRoot)
    Write-AxiomDiag -Message ("approve with: -Apply -ApproveDigest {0}" -f $planDigest)
    $envelope.dry_run = $true
    $envelope.mutated = $false
    $envelope.service_registration = [ordered]@{
        kind       = $serviceKind
        task_name  = $(if ($serviceTaskName) { $serviceTaskName } else { $null })
        owner      = 'axiom-cli'
        registered = ($serviceKind -ne 'none' -and -not [string]::IsNullOrEmpty($serviceTaskName))
        removed    = $false
        reason     = 'planning only: the recorded managed-service registration is not removed until this plan is approved'
    }
    $envelope.preserved = @([ordered]@{
            path   = $resolvedRoot
            reason = 'planning only: nothing was removed and nothing was touched'
        })
    Complete-AxiomRun -Envelope $envelope -Code $exitOk -Outcome 'planned' -Status 'ok' `
        -Message ("planned removal of release {0}; nothing was changed" -f $installedVersion)
}

if ([string]::IsNullOrEmpty($ApproveDigest)) {
    Add-AxiomRefusal -Envelope $envelope -Check 'approval-digest' -Artifact $resolvedRoot `
        -Expected $planDigest -Reason 'a mutating uninstall requires -ApproveDigest bound to the uninstall-plan sha256'
    Complete-AxiomRun -Envelope $envelope -Code $exitValidation -Outcome 'not_removed' -Status 'refused' `
        -Message 'missing -ApproveDigest: a mutating uninstall must be approved'
}

$approved = $ApproveDigest.Trim().ToLowerInvariant()
$envelope.approved_digest = $approved
if ($approved -ne $planDigest) {
    Add-AxiomRefusal -Envelope $envelope -Check 'approval-digest' -Artifact $resolvedRoot `
        -Expected $planDigest -Actual $approved -Reason 'the approval digest does not match the uninstall-plan sha256'
    Complete-AxiomRun -Envelope $envelope -Code $exitAuthorization -Outcome 'not_removed' -Status 'refused' `
        -Message ("approval digest mismatch: expected {0}, got {1}" -f $planDigest, $approved)
}

# ---------------------------------------------------------------------------
# 3. Commit the removal.
# ---------------------------------------------------------------------------

$transactionId = New-AxiomTransactionId
$envelope.transaction_id = $transactionId
$lockPath = Get-AxiomLockPath $resolvedRoot
$lockStream = Enter-AxiomTransactionLock -Path $lockPath -TransactionId $transactionId
if ($null -eq $lockStream) {
    Complete-AxiomRun -Envelope $envelope -Code $exitLock -Outcome 'not_removed' -Status 'refused' -Retryable $true `
        -Message ("another axiom-cli transaction holds the lock at {0}; retry when it finishes" -f $lockPath)
}

try {
    $machinePathBefore = Get-AxiomMachinePathSnapshot
    $removedList = New-Object System.Collections.Generic.List[object]

    # An interrupted install must be recovered by the installer before it is removed again.
    $journalPath = Get-AxiomJournalPath $resolvedRoot
    if (Test-Path -LiteralPath $journalPath -PathType Leaf) {
        Add-AxiomRefusal -Envelope $envelope -Check 'install-journal-present' -Artifact $journalPath `
            -Reason 'an interrupted install is journaled here; re-run Install-AxiomCli.ps1 so it can recover before uninstalling'
        Complete-AxiomRun -Envelope $envelope -Code $exitConflict -Outcome 'not_removed' -Status 'refused' `
            -Message ("refusing to uninstall over an interrupted install: {0}" -f $journalPath)
    }

    # --- entrypoint: remove only a file this install recorded and that still matches ----
    $recordedEntrySha = ''
    foreach ($stateArtifact in $stateArtifacts) {
        if ([string](Get-AxiomProp -Object $stateArtifact -Name 'name' -Default '') -eq $entrypointName) {
            $recordedEntrySha = [string](Get-AxiomProp -Object $stateArtifact -Name 'sha256' -Default '')
        }
    }
    if ($entrypointExists) {
        if (-not $entrypointIsOurs) {
            Add-AxiomRefusal -Envelope $envelope -Check 'entrypoint-ownership' -Artifact $entrypointPath `
                -Reason 'the file at the entrypoint path is not the one this install recorded; refusing to delete it'
            Complete-AxiomRun -Envelope $envelope -Code $exitConflict -Outcome 'not_removed' -Status 'refused' `
                -Message ("refusing to remove an unowned file: {0}" -f $entrypointPath)
        }
        if (-not [string]::IsNullOrEmpty($recordedEntrySha) -and $entrypointSha -ne $recordedEntrySha) {
            Add-AxiomRefusal -Envelope $envelope -Check 'entrypoint-sha256' -Artifact $entrypointPath `
                -Expected $recordedEntrySha -Actual $entrypointSha `
                -Reason 'the installed entrypoint changed since the install; refusing to delete a file this transaction did not put there'
            Complete-AxiomRun -Envelope $envelope -Code $exitConflict -Outcome 'not_removed' -Status 'refused' `
                -Message ("refusing to remove a modified entrypoint: {0}" -f $entrypointPath)
        }
        Remove-Item -LiteralPath $entrypointPath -Force
        $removedList.Add([ordered]@{ path = $entrypointPath; kind = 'executable' })
    }

    if (-not [string]::IsNullOrEmpty($ownedGeneration)) {
        Remove-Item -LiteralPath $ownedGeneration -Recurse -Force -ErrorAction SilentlyContinue
        if (-not (Test-Path -LiteralPath $ownedGeneration)) {
            $removedList.Add([ordered]@{ path = $ownedGeneration; kind = 'generation' })
        }
    }

    $stagingDir = Get-AxiomStagingDir $resolvedRoot
    if (Test-Path -LiteralPath $stagingDir) {
        Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue
        if (-not (Test-Path -LiteralPath $stagingDir)) {
            $removedList.Add([ordered]@{ path = $stagingDir; kind = 'staging' })
        }
    }

    # --- per-user PATH entry: remove exactly the entry this install recorded -----------
    $pathRuleApplied = $false
    if (-not [string]::IsNullOrEmpty($recordedPathEntry)) {
        $pathBefore = Get-AxiomUserPathSnapshot
        if (Test-AxiomPathContains -Value ([string]$pathBefore.value) -Entry $recordedPathEntry) {
            $newPathValue = Remove-AxiomPathEntry -Value ([string]$pathBefore.value) -Entry $recordedPathEntry
            Set-AxiomUserPathValue -Value $newPathValue -Kind ([string]$pathBefore.kind)
            $removedList.Add([ordered]@{ path = $recordedPathEntry; kind = 'path-entry' })
            $pathRuleApplied = $true
        }
    }

    # --- managed service registration -------------------------------------------------
    $serviceRemoved = $false
    if ($serviceKind -eq 'scheduled-task-at-logon' -and -not [string]::IsNullOrEmpty($serviceTaskName)) {
        try {
            Unregister-ScheduledTask -TaskName $serviceTaskName -Confirm:$false -ErrorAction Stop
            $removedList.Add([ordered]@{ path = $serviceTaskName; kind = 'service-registration' })
            $serviceRemoved = $true
        } catch {
            $envelope.limitations = @($envelope.limitations) + @(
                ("the recorded service registration '{0}' could not be removed: {1}" -f $serviceTaskName, [string]$_.Exception.Message)
            )
        }
    }

    # --- shared install manifest: drop this install's rows, keep other components' ----
    $manifestPathValue = Get-AxiomManifestPath $resolvedRoot
    $manifestBefore = Read-AxiomJsonFile -Path $manifestPathValue
    if ($null -ne $manifestBefore) {
        $ours = New-Object System.Collections.Generic.List[string]
        foreach ($stateArtifact in $stateArtifacts) {
            $ours.Add([string](Get-AxiomProp -Object $stateArtifact -Name 'component' -Default ''))
        }
        $kept = New-Object System.Collections.Generic.List[object]
        foreach ($manifestComponent in @(Get-AxiomProp -Object $manifestBefore -Name 'components' -Default @())) {
            $name = [string](Get-AxiomProp -Object $manifestComponent -Name 'component' -Default '')
            if ($ours -contains $name) { continue }
            $kept.Add($manifestComponent)
        }
        if ($kept.Count -eq 0) {
            Remove-Item -LiteralPath $manifestPathValue -Force -ErrorAction SilentlyContinue
        } else {
            Write-AxiomJsonFile -Path $manifestPathValue -Value ([ordered]@{
                    schema_version = 1
                    document_kind  = 'axiom-cli-install-manifest'
                    platform       = $script:AxiomPlatformId
                    host           = (Get-AxiomHostInfo)
                    updated_at     = (Get-AxiomTimestamp)
                    components     = @($kept.ToArray())
                    preserved_roots = @('workspace state and the per-project graph output root (<project>\.axiom\graph) live outside this install root')
                })
        }
    }

    # --- state ------------------------------------------------------------------------
    Remove-Item -LiteralPath $statePath -Force -ErrorAction SilentlyContinue

    # Every mutation is done. Release the lock before removing any directory that contains it:
    # on Windows an open lock file cannot be deleted, so a purge that removed the root first
    # would fail silently and leave the install root behind.
    Exit-AxiomTransactionLock -Stream $lockStream -Path $lockPath
    $lockStream = $null

    $purgedRoot = $false
    if ($purge) {
        if (Test-Path -LiteralPath $resolvedRoot) {
            Remove-Item -LiteralPath $resolvedRoot -Recurse -Force -ErrorAction SilentlyContinue
            if (-not (Test-Path -LiteralPath $resolvedRoot)) {
                $removedList.Add([ordered]@{ path = $resolvedRoot; kind = 'install-root' })
                $purgedRoot = $true
            }
        }
    } elseif (Test-Path -LiteralPath $cliDir) {
        Remove-Item -LiteralPath $cliDir -Recurse -Force -ErrorAction SilentlyContinue
        if (-not (Test-Path -LiteralPath $cliDir)) {
            $removedList.Add([ordered]@{ path = $cliDir; kind = 'state-directory' })
        }
    }

    $machinePathAfter = Get-AxiomMachinePathSnapshot
    if ($machinePathBefore -ne $machinePathAfter) {
        throw 'the machine-wide PATH changed during this transaction'
    }

    # --- report -----------------------------------------------------------------------
    $envelope.dry_run = $false
    $envelope.mutated = $true
    $envelope.path_rule.scope = 'user'
    $envelope.path_rule.entry = $recordedPathEntry
    $envelope.path_rule.applied = $false
    $envelope.path_rule.previous_present = ($null -ne $recordedPathEntry -and $recordedPathEntry -ne '')
    $envelope.path_rule.machine_wide_change = $false
    $envelope.service_registration = [ordered]@{
        kind       = $serviceKind
        task_name  = $(if ($serviceTaskName) { $serviceTaskName } else { $null })
        owner      = 'axiom-cli'
        registered = $false
        removed    = $serviceRemoved
        reason     = 'the recorded managed-service registration is removed by uninstall; the daemon itself is owned by axiom-graphd'
    }
    $envelope.removed = @($removedList.ToArray())

    $preservedRows = New-Object System.Collections.Generic.List[object]
    if (-not $purgedRoot) {
        $preservedRows.Add([ordered]@{
                path   = $resolvedRoot
                reason = 'the per-user install root is preserved by default; pass -PurgeData (a separate plan digest) to remove it'
            })
    }
    $preservedRows.Add([ordered]@{
            path   = '<project>\.axiom\graph'
            reason = 'the per-project graph output root is user data; uninstall never touches it and never recursively deletes .axiom'
        })
    if ($null -ne $manifestBefore) {
        $preservedRows.Add([ordered]@{
                path   = $manifestPathValue
                reason = 'the shared per-user install manifest is preserved when other components still have rows in it'
            })
    }
    $envelope.preserved = @($preservedRows.ToArray())

    $outcome = 'removed'
    $summary = ("removed release {0} from {1}: {2} item(s) removed, user data preserved" -f $installedVersion, $resolvedRoot, $removedList.Count)
    Complete-AxiomRun -Envelope $envelope -Code $exitOk -Outcome $outcome -Status 'ok' -Message $summary
} catch {
    Add-AxiomRefusal -Envelope $envelope -Check 'uninstall' -Artifact $resolvedRoot -Reason ([string]$_.Exception.Message)
    Complete-AxiomRun -Envelope $envelope -Code $exitIo -Outcome 'not_removed' -Status 'error' -Retryable $true `
        -Message ("uninstall failed: {0}" -f [string]$_.Exception.Message)
} finally {
    # Safety net: the lock is normally released before the state directory is removed.
    Exit-AxiomTransactionLock -Stream $lockStream -Path $lockPath
}
