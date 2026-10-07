# Builds the managed half, copies the native library next to it, and runs the log-crossing matrix
# on the P-core threads, three interleaved rounds, into ../results/dotnet-cost.txt.
$here = $PSScriptRoot
$dll = Join-Path $here "..\..\target\release\observability_spike.dll"
dotnet build -c Release --no-restore $here | Out-Null
foreach ($tfm in "net8.0", "net48") { Copy-Item $dll (Join-Path $here "bin\Release\$tfm\") -Force }
(Get-Process -Id $PID).ProcessorAffinity = [IntPtr]255
$out = Join-Path $here "..\results\dotnet-cost.txt"
Remove-Item $out -ErrorAction SilentlyContinue
foreach ($round in 1..3) {
  foreach ($threads in 1, 4) {
    foreach ($strategy in "native", "noop", "blob", "decode") {
      & dotnet (Join-Path $here "bin\Release\net8.0\LogPumpSpike.dll") log $strategy 500000 $threads | Add-Content $out
      & (Join-Path $here "bin\Release\net48\LogPumpSpike.exe") log $strategy 500000 $threads | Add-Content $out
    }
  }
}
& dotnet (Join-Path $here "bin\Release\net8.0\LogPumpSpike.dll") metrics | Add-Content $out
& (Join-Path $here "bin\Release\net48\LogPumpSpike.exe") metrics | Add-Content $out
Get-Content $out
