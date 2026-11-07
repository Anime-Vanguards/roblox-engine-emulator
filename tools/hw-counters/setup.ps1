<#
.SYNOPSIS
Installs (or removes) ree's counter service: the one elevated step after which `ree ... --profile hw` reads the CPU's
hardware counters from an ordinary process.

.DESCRIPTION
Run once from an elevated PowerShell. It builds tools/hw-counters (a small Rust program with no dependencies) in a
temporary folder, copies it to "%ProgramFiles%\ree-hw-counters" (which only administrators may change), and registers
it as the Windows service "ree-hw-counters". What that grants:

  - The service runs as LocalSystem holding two privileges only: the system's profiling right
    (SeSystemProfilePrivilege, what a kernel trace session with the CPU's counters needs) and the right to traverse
    folders (SeChangeNotifyPrivilege, which every process has).
  - It starts on demand. The account given (default: the one running this script) may start it and read its status;
    nothing else (not stop, reconfigure or delete it).
  - It serves one named pipe that only that account and SYSTEM may open, from this machine only. The client process
    must run as that account; it gets counter readings for its own threads only, never another process's.
  - While a client counts (30 minutes at most), it runs one kernel trace session of context switches carrying the
    counters' values. It stops itself after a minute with no client.
  - On AMD Family 1Ah (Zen 5) CPUs it registers four cache events with Windows (through wpr.exe, until the next
    restart) so the L1d, L2 and last-level cache misses can be counted.

Nothing else on the machine changes: no driver, no group membership, no policy. Then it runs the service's probe,
which checks every counter against kernels whose counts are known.

.PARAMETER User
The account that may use the service (a name or a SID). Default: the account running this script.

.PARAMETER Binary
A built ree-hw-counters.exe to install instead of building one (otherwise cargo must be on PATH).

.PARAMETER Uninstall
Stops and removes the service and its folder.
#>
param([string]$User, [string]$Binary, [switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$name = 'ree-hw-counters'
$folder = Join-Path $env:ProgramFiles $name
$exe = Join-Path $folder "$name.exe"
$key = "HKLM:\SYSTEM\CurrentControlSet\Services\$name"

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host 'This needs an elevated PowerShell (right-click PowerShell, Run as administrator), then:'
    Write-Host "  & '$PSCommandPath'"
    exit 1
}

function Remove-CounterService {
    $existing = Get-Service -Name $name -ErrorAction SilentlyContinue
    if ($existing) {
        if ($existing.Status -ne 'Stopped') {
            Stop-Service -Name $name -Force -ErrorAction SilentlyContinue
            $existing.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
        & sc.exe delete $name | Out-Null
        # (the service manager deletes a service once no handle to it is open)
        for ($i = 0; $i -lt 100 -and (Get-Service -Name $name -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 100 }
    }
}

if ($Uninstall) {
    Remove-CounterService
    if (Test-Path -LiteralPath $folder) { Remove-Item -LiteralPath $folder -Recurse -Force }
    Write-Host "Removed the $name service and $folder. (Counters it registered with Windows go at the next restart.)"
    exit 0
}

# the account that may use it
if ($User) {
    if ($User -match '^S-1-') { $sid = $User }
    else { $sid = (New-Object Security.Principal.NTAccount($User)).Translate([Security.Principal.SecurityIdentifier]).Value }
} else {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
}
$account = (New-Object Security.Principal.SecurityIdentifier($sid)).Translate([Security.Principal.NTAccount]).Value

# the program: built from this checkout in a temporary folder (nothing is written in the checkout), unless given
$build = $null
if (-not $Binary) {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) { throw 'cargo is not on PATH: install Rust (rustup), or pass -Binary <a built ree-hw-counters.exe>' }
    $build = Join-Path $env:TEMP ("$name-build-" + [guid]::NewGuid().ToString('N'))
    Write-Host "Building $name in $build ..."
    & $cargo.Source build --release --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --target-dir $build
    if ($LASTEXITCODE -ne 0) { throw "building $name failed" }
    $Binary = Join-Path $build "release\$name.exe"
}
if (-not (Test-Path -LiteralPath $Binary)) { throw "no program at $Binary" }

Remove-CounterService
New-Item -ItemType Directory -Force -Path $folder | Out-Null
Copy-Item -LiteralPath $Binary -Destination $exe -Force
if ($build) { Remove-Item -LiteralPath $build -Recurse -Force }

New-Service -Name $name -BinaryPathName ('"' + $exe + '" service --allow ' + $sid) -DisplayName 'ree hardware counters' -StartupType Manual `
    -Description "Counts CPU hardware events (instructions, cycles, cache and branch misses) for ree's --profile hw, for processes of $account and only for their own threads. Starts on demand, stops itself when idle." | Out-Null
# LocalSystem with only the profiling right (and folder traversal)
& sc.exe privs $name 'SeSystemProfilePrivilege/SeChangeNotifyPrivilege' | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'limiting the service to the profiling right failed' }
# the account may start it and read its status (RP: start, LC: query status), beside the service's default access
$sd = ((& sc.exe sdshow $name) -join '').Trim()
if ($sd -notmatch '^D:') { throw "reading the service's security descriptor: $sd" }
$sd = $sd -replace '^D:([A-Z]*)', ('D:$1(A;;RPLC;;;' + $sid + ')')
& sc.exe sdset $name $sd | Out-Null
if ($LASTEXITCODE -ne 0) { throw "setting the service's security descriptor failed ($sd)" }
Write-Host "Installed $exe as the $name service for $account ($sid)."

# the check: the probe counts known kernels through the service, as the account's processes will
function Invoke-Probe {
    Write-Host ''
    $lines = & $exe probe
    $exitCode = $LASTEXITCODE
    foreach ($line in $lines) { Write-Host $line }
    return @{ Code = $exitCode; Text = ($lines -join "`n") }
}
$probe = Invoke-Probe
if ($probe.Code -eq 2 -and $probe.Text -match 'error 5\b|1314|privilege|access denied|profiling right|0xc558063a') {
    # (a right the trace session needs beyond the profiling right: LocalSystem's own set, and the check again)
    Write-Host 'The service lacked a right it needed: giving it LocalSystem''s own privileges and checking again.'
    Stop-Service -Name $name -Force -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $key -Name RequiredPrivileges -ErrorAction SilentlyContinue
    $probe = Invoke-Probe
}
$code = $probe.Code
Write-Host ''
if ($code -eq 0) {
    Write-Host "Done. Processes of $account now read the CPU's counters through the service:"
    Write-Host '  ree run|record ... --profile hw [--profile-counters a,b]   (docs/cli.md "Hardware counters")'
} elseif ($code -eq 1) {
    Write-Host 'Installed, but some counters did not count what their names say (above): the profile reports them as they are.'
} else {
    Write-Host 'Installed, but the service could not count (above). The service stays installed; remove it with:'
}
Write-Host "  Remove: & '$PSCommandPath' -Uninstall"
exit $code
