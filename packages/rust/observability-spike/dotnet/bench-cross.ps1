# The crossing alone: native calls the managed callback in a tight loop (no event rendered). Three
# interleaved rounds on the P-core threads, into ../results/dotnet-cross.txt.
$here = $PSScriptRoot
$dll = Join-Path $here "..\..\target\release\observability_spike.dll"
dotnet build -c Release --no-restore $here | Out-Null
foreach ($tfm in "net8.0", "net48") { Copy-Item $dll (Join-Path $here "bin\Release\$tfm\") -Force }
(Get-Process -Id $PID).ProcessorAffinity = [IntPtr]255
$out = Join-Path $here "..\results\dotnet-cross.txt"
Remove-Item $out -ErrorAction SilentlyContinue
foreach ($round in 1..3) {
  foreach ($strategy in "noop", "blob", "batch", "decode") {
    foreach ($n in 20000, 1000000) {
      & dotnet (Join-Path $here "bin\Release\net8.0\LogPumpSpike.dll") cross $strategy $n | Add-Content $out
      & (Join-Path $here "bin\Release\net48\LogPumpSpike.exe") cross $strategy $n | Add-Content $out
    }
  }
}
$exe = Join-Path $here "..\..\target\release\examples\emit_native.exe"
foreach ($round in 1..2) { & $exe | Add-Content $out }
Get-Content $out
