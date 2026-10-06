# The benchmark campaign benchmarks.md reports: every framework, just-in-time and ahead-of-time,
# each transport, idle and busy, the runs of a pass interleaved, and the median of the passes.
#
# Run from a Visual Studio x64 developer PowerShell; -Out is relative to the current directory, the
# rest to this script's. On a hybrid processor, pass the performance cores as -Affinity (0xFF where
# they are logical processors 0-7): a run the scheduler spreads over both kinds of core measures
# where its threads landed.
param(
  [int] $Passes = 3,
  [string[]] $Frameworks = @("net4.8", "net8.0", "net10.0"),
  [string[]] $Transports = @("native", "managed"),
  [string[]] $Scenarios = @("idle", "busy"),
  # Ahead-of-time variants of the frameworks above but net4.8, published with `dotnet publish`.
  [switch] $Aot,
  [string] $Affinity = "",
  [string] $Priority = "",
  # The dotnet that builds; it has to know every framework in -Frameworks.
  [string] $Dotnet = "dotnet",
  # Where the runtimes of -DotnetRootFor live, when not where `dotnet` is installed system-wide;
  # the apphost of every other framework looks there instead, so they are left out.
  [string] $DotnetRoot = "",
  [string[]] $DotnetRootFor = @("net10.0", "net11.0"),
  # Passed to the build and every publish, e.g. -p:NativeEngineH2Batch=true for the engine
  # native-batch needs.
  [string[]] $BuildArgs = @(),
  # Passed to every publish only, e.g. -p:IlcUseEnvironmentalTools=true to link with the
  # developer prompt's tools.
  [string[]] $PublishArgs = @(),
  [string] $Out = "campaign.log",
  [switch] $NoBuild
)
# Not Stop: Windows PowerShell turns what a native tool writes to stderr into a terminating error.
# Exit codes are checked instead.
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
  ARMONIK_BENCH_PRIORITY = $env:ARMONIK_BENCH_PRIORITY
  DOTNET_ROOT = $env:DOTNET_ROOT
}
function Set-Environment([string] $name, [string] $value) {
  if ($value) { Set-Item "Env:$name" $value } else { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
}

Set-Content -Path $Out -Encoding UTF8 -Value "# campaign $(Get-Date -Format s) affinity=$Affinity priority=$Priority"
try {
  Set-Environment ARMONIK_BENCH_AFFINITY $Affinity
  Set-Environment ARMONIK_BENCH_PRIORITY $Priority
  for ($pass = 1; $pass -le $Passes; $pass++) {
    foreach ($scenario in $Scenarios) {
      foreach ($variant in $variants) {
        foreach ($transport in $Transports) {
          $arguments = @($transport)
          if ($scenario -eq "busy") { $arguments += "busy" }
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
$runs = Get-Content $Out -Encoding UTF8 | Where-Object { $_ -match '^pass=\d+ (\S+) (\S+)( busy)? (.*)$' } | ForEach-Object {
  $fields = @{}
  foreach ($pair in ($Matches[4] -split ' ')) {
    $key, $value = $pair -split '=', 2
    $fields[$key] = [double]::Parse($value, [Globalization.CultureInfo]::InvariantCulture)
  }
  [pscustomobject]@{
    Transport = $Matches[1]
    Variant = $Matches[2]
    Scenario = if ($Matches[3]) { "busy" } else { "idle" }
    Fields = $fields
  }
}
function Median([double[]] $values) {
  if ($values.Count -eq 0) { return "" }
  $sorted = @($values | Sort-Object)
  $middle = [int][math]::Floor($sorted.Count / 2)
  if ($sorted.Count % 2 -eq 1) { return [math]::Round($sorted[$middle]) }
  return [math]::Round(($sorted[$middle - 1] + $sorted[$middle]) / 2)
}
$columns = "p50_us", "p95_us", "p99_us", "stream_mib_s", "upload_stream_mib_s", "upload_unary_mib_s", "alloc_b_call", "cpu_us_call",
           "upload_stream_cpu_us_mib", "upload_unary_cpu_us_mib"
""
"| Variant | Scenario | Transport | " + ($columns -join " | ") + " |"
"|---|---|---|" + (($columns | ForEach-Object { "---:" }) -join "|") + "|"
$runs | Group-Object Variant, Scenario, Transport | ForEach-Object {
  $first = $_.Group[0]
  if ($_.Count -lt $Passes) {
    Write-Warning "$($first.Variant) $($first.Scenario) $($first.Transport): the median of $($_.Count) runs, not $Passes"
  }
  $cells = foreach ($column in $columns) {
    Median ($_.Group | ForEach-Object { if ($_.Fields.ContainsKey($column)) { $_.Fields[$column] } })
  }
  "| $($first.Variant) | $($first.Scenario) | $($first.Transport) | " + ($cells -join " | ") + " |"
}
