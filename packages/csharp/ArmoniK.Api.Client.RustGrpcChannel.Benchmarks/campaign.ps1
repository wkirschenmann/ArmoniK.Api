# The benchmark campaign benchmarks.md reports: every framework, just-in-time and ahead-of-time,
# each transport, in each scenario, the runs of a pass interleaved, and the median of the passes.
#
# Run from a Visual Studio x64 developer PowerShell; -Out is relative to the current directory, the
# rest to this script's. On a hybrid processor, pass the performance cores as -Affinity (0xFF where
# they are logical processors 0-7): a run the scheduler spreads over both kinds of core measures
# where its threads landed. -ServerAffinity gives the server cores of its own, sharing none with
# -Affinity so that client and server never compete for one; it needs -Affinity. Without
# -ServerAffinity the server takes -Affinity.
param(
  [int] $Passes = 3,
  [string[]] $Frameworks = @("net4.8", "net8.0", "net10.0"),
  [string[]] $Transports = @("native", "managed"),
  # idle, busy, and pinned and shared: the unary latency on one connection, without and with a
  # streamed upload beside it, which the native transports alone run. gzip, zstd, concurrent and
  # large: compression both ways, 16 and 64 calls in flight, unary messages of 4 and 16 MiB; zstd
  # runs on the native transports, against a server ARMONIK_BENCH_SERVER names (benchmarks.md).
  # mixed: large uploads and unary calls at once over mutual TLS, its parameters after it, space
  # separated, e.g. "mixed k=4 m=64 pool=16"; a pool other than server runs on the native transports.
  [string[]] $Scenarios = @("idle", "busy"),
  # Ahead-of-time variants of the frameworks above but net4.8, published with `dotnet publish`.
  [switch] $Aot,
  [string] $Affinity = "",
  [string] $ServerAffinity = "",
  [string] $Priority = "",
  # The dotnet that builds; it has to know every framework in -Frameworks.
  [string] $Dotnet = "dotnet",
  # Where the runtimes of -DotnetRootFor live, when not where `dotnet` is installed system-wide;
  # the apphost of every other framework looks there instead, so they are left out.
  [string] $DotnetRoot = "",
  [string[]] $DotnetRootFor = @("net10.0", "net11.0"),
  # Passed to the build and every publish, e.g. -p:NativeEngineH2Batch=false for an engine on
  # stock h2.
  [string[]] $BuildArgs = @(),
  # Passed to every publish only, e.g. -p:IlcUseEnvironmentalTools=true to link with the
  # developer prompt's tools.
  [string[]] $PublishArgs = @(),
  [string] $Out = "campaign.log",
  [switch] $NoBuild
)
# Not Stop: Windows PowerShell turns what a native tool writes to stderr into a terminating error.
# Exit codes are checked instead.

# Every run would refuse these masks; refuse them once, before the build.
function Read-Mask([string] $name, [string] $mask) {
  $number = 0L
  if (-not [long]::TryParse(($mask -replace '^0x', ''), [Globalization.NumberStyles]::HexNumber, [Globalization.CultureInfo]::InvariantCulture, [ref] $number)) {
    throw "$name is $mask, not a mask of logical processors in hexadecimal"
  }
  $number
}
if ($ServerAffinity) {
  if (-not $Affinity) { throw "-ServerAffinity needs -Affinity, or the benchmark runs unpinned" }
  if ((Read-Mask "-Affinity" $Affinity) -band (Read-Mask "-ServerAffinity" $ServerAffinity)) {
    throw "-Affinity $Affinity and -ServerAffinity $ServerAffinity share logical processors"
  }
}

$here = $PSScriptRoot
$project = Join-Path $here "ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.csproj"
$name = "ArmoniK.Api.Client.RustGrpcChannel.Benchmarks"
$staging = Join-Path $here "bin\campaign"

# Each variant: the executable that runs it, and the DOTNET_ROOT its apphost needs.
$variants = @()
foreach ($framework in $Frameworks) {
  $root = if ($DotnetRootFor -contains $framework) { $DotnetRoot } else { "" }
  $variants += [pscustomobject]@{ Exe = Join-Path $here "bin\Release\$framework\$name.exe"; Root = $root }
  if ($Aot -and $framework -ne "net4.8") {
    $variants += [pscustomobject]@{ Exe = Join-Path $staging "$framework-aot\$name.exe"; Root = "" }
  }
}

if (-not $NoBuild) {
  & $Dotnet build $project -c Release @BuildArgs
  if ($LASTEXITCODE -ne 0) { throw "the build failed" }
  if ($Aot) {
    foreach ($framework in $Frameworks | Where-Object { $_ -ne "net4.8" }) {
      & $Dotnet publish $project -c Release -f $framework -r win-x64 -p:BenchmarkAot=true @BuildArgs @PublishArgs -o (Join-Path $staging "$framework-aot")
      if ($LASTEXITCODE -ne 0) { throw "the $framework ahead-of-time publish failed" }
    }
  }
}

foreach ($variant in $variants) {
  if (-not (Test-Path $variant.Exe)) { throw "no $($variant.Exe); build it, or drop -NoBuild" }
}

# Put back once the runs are done, so the shell's next run is not pinned by this one.
$saved = @{
  ARMONIK_BENCH_AFFINITY = $env:ARMONIK_BENCH_AFFINITY
  ARMONIK_BENCH_SERVER_AFFINITY = $env:ARMONIK_BENCH_SERVER_AFFINITY
  ARMONIK_BENCH_PRIORITY = $env:ARMONIK_BENCH_PRIORITY
  DOTNET_ROOT = $env:DOTNET_ROOT
}
function Set-Environment([string] $name, [string] $value) {
  if ($value) { Set-Item "Env:$name" $value } else { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
}

Set-Content -Path $Out -Encoding UTF8 -Value "# campaign $(Get-Date -Format s) affinity=$Affinity server_affinity=$ServerAffinity priority=$Priority"
try {
  Set-Environment ARMONIK_BENCH_AFFINITY $Affinity
  Set-Environment ARMONIK_BENCH_SERVER_AFFINITY $ServerAffinity
  Set-Environment ARMONIK_BENCH_PRIORITY $Priority
  for ($pass = 1; $pass -le $Passes; $pass++) {
    foreach ($scenario in $Scenarios) {
      foreach ($variant in $variants) {
        foreach ($transport in $Transports) {
          if (($scenario -in @("pinned", "shared", "zstd")) -and ($transport -notlike "native*")) { continue }
          if (($scenario -match ' pool=\d') -and ($transport -notlike "native*")) { continue }
          $arguments = @($transport)
          if ($scenario -ne "idle") { $arguments += -split $scenario }
          Set-Environment DOTNET_ROOT $(if ($variant.Root) { $variant.Root } else { $saved.DOTNET_ROOT })
          $line = & $variant.Exe @arguments 2>&1 | Select-Object -Last 1
          if ($LASTEXITCODE -ne 0) {
            Write-Warning "pass $pass, $($variant.Exe) $($arguments -join ' ') exited $LASTEXITCODE`: $line"
            continue
          }
          # Not Tee-Object: Windows PowerShell appends UTF-16 to a file it did not start.
          "pass=$pass $line"
          Add-Content -Path $Out -Encoding UTF8 -Value "pass=$pass $line"
        }
      }
    }
  }
}
finally {
  foreach ($name in $saved.Keys) { Set-Environment $name $saved[$name] }
}

# The median of the passes, per transport, variant and scenario.
$runs = Get-Content $Out -Encoding UTF8 | Where-Object { $_ -match '^pass=\d+ (\S+) (\S+)( busy| pinned| shared| gzip| zstd| concurrent| large| mixed-\S+)? (.*)$' } | ForEach-Object {
  # In the order the line gives them, so each scenario's table keeps the benchmark's columns.
  $fields = [ordered]@{}
  foreach ($pair in ($Matches[4] -split ' ')) {
    $key, $value = $pair -split '=', 2
    $number = 0.0
    if (-not [double]::TryParse($value, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref] $number)) {
      throw "garbled field '$pair' in: $_"
    }
    $fields[$key] = $number
  }
  [pscustomobject]@{
    Transport = $Matches[1]
    Variant = $Matches[2]
    Scenario = if ($Matches[3]) { $Matches[3].Trim() } else { "idle" }
    Fields = $fields
  }
}
function Median([double[]] $values) {
  if ($values.Count -eq 0) { return "" }
  $sorted = @($values | Sort-Object)
  $middle = [int][math]::Floor($sorted.Count / 2)
  $median = if ($sorted.Count % 2 -eq 1) { $sorted[$middle] } else { ($sorted[$middle - 1] + $sorted[$middle]) / 2 }
  # A tenth below 100, where a whole unit is a large share: the CPU per call of calls in flight.
  if ([math]::Abs($median) -lt 100) { return [math]::Round($median, 1) }
  return [math]::Round($median)
}
$latency = "p50_us", "p95_us", "p99_us", "stream_mib_s", "upload_stream_mib_s", "upload_unary_mib_s", "alloc_b_call", "cpu_us_call",
           "upload_stream_cpu_us_mib", "upload_unary_cpu_us_mib", "upload_beside_mib_s", "calib_us", "calib_end_us"
# One table per scenario: the others report figures of their own, in the order their lines give them.
# The mixed runs share one, a row per set of parameters, which the line names (mixed-k4-m64-...).
$runs | Group-Object { $_.Scenario -replace '^mixed-.*', 'mixed' } | ForEach-Object {
  # Every figure any run gave, in the order the lines first give them: a large run's downloads
  # are the native transports' alone.
  $columns = if ($_.Name -in @("idle", "busy", "pinned", "shared")) { $latency } else {
    $seen = [ordered]@{}
    foreach ($run in $_.Group) { foreach ($key in $run.Fields.Keys) { $seen[$key] = $true } }
    @($seen.Keys)
  }
  ""
  "| Variant | Scenario | Transport | " + ($columns -join " | ") + " |"
  "|---|---|---|" + (($columns | ForEach-Object { "---:" }) -join "|") + "|"
  $_.Group | Group-Object Variant, Transport, Scenario | ForEach-Object {
    $first = $_.Group[0]
    if ($_.Count -lt $Passes) {
      Write-Warning "$($first.Variant) $($first.Scenario) $($first.Transport): the median of $($_.Count) runs, not $Passes"
    }
    $cells = foreach ($column in $columns) {
      Median ($_.Group | ForEach-Object { if ($_.Fields.Contains($column)) { $_.Fields[$column] } })
    }
    "| $($first.Variant) | $($first.Scenario) | $($first.Transport) | " + ($cells -join " | ") + " |"
  }
}
