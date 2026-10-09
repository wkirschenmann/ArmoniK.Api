using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>Scratch instrumentation: the timestamps of one sequential call's checkpoints.</summary>
public static class Probe
{
  public const int Points = 56;
  public const int RustBase = 32;
  public const int RustPoints = 24;

  public static readonly string[] Names = new string[Points];

  static Probe()
  {
    var named = new (int, string)[]
    {
      (0, "C bench: before call"),
      (1, "C ak_call_start returned"),
      (12, "C sender: before serializer"),
      (13, "C before ak_get_call_buffer"),
      (14, "C ak_get_call_buffer returned"),
      (2, "C serialized, before ak_call_send_message"),
      (3, "C ak_call_send_message returned"),
      (4, "C first OnEvent entry"),
      (5, "C OnEvent entry, batch with status"),
      (6, "C Arrived() done (status batch)"),
      (7, "C OnEvent returns (status batch)"),
      (8, "C reader resumes after arrival"),
      (15, "C head taken"),
      (16, "C message deserialized"),
      (17, "C status decoded"),
      (18, "C status resolved"),
      (19, "C ReleaseMany (ak_events_consumed) done"),
      (20, "C reader published finished"),
      (9, "C reader parsed, before Settled"),
      (10, "C Reduced returns"),
      (11, "C bench: after await"),
      (RustBase + 0, "R ak_call_start entry"),
      (RustBase + 1, "R start: looked up, decoded"),
      (RustBase + 2, "R start: call prepared"),
      (RustBase + 3, "R start: registered"),
      (RustBase + 4, "R ak_call_send_message entry"),
      (RustBase + 5, "R commit: before spawn"),
      (RustBase + 6, "R commit: spawned"),
      (RustBase + 7, "R driver: first poll"),
      (RustBase + 8, "R driver: before streaming()"),
      (RustBase + 9, "R driver: response head"),
      (RustBase + 10, "R driver: first message"),
      (RustBase + 11, "R driver: trailers"),
      (RustBase + 12, "R host callback (first)"),
      (RustBase + 13, "R driver: head built, before staging"),
      (RustBase + 14, "R driver: head staged, finish entered"),
      (RustBase + 15, "R read gate admitted (first)"),
      (RustBase + 16, "R sink: message staged (first)"),
      (RustBase + 17, "R end: entered"),
      (RustBase + 18, "R end: writer done"),
      (RustBase + 19, "R end: status staged"),
      (RustBase + 20, "R ak_events_consumed entry"),
      (RustBase + 21, "R payload returned, before moved_on (last)"),
      (RustBase + 22, "R moved_on done (last)"),
      (RustBase + 23, "R ak_events_consumed exit"),
    };
    foreach (var (at, name) in named)
    {
      Names[at] = name;
    }
  }

  public static readonly long[] At = new long[Points];

  [DllImport("armonik_transport_ffi")]
  private static extern long ak_probe_at(uint point);

  [DllImport("armonik_transport_ffi")]
  private static extern void ak_probe_reset();

  public static void Reset()
  {
    for (var i = 0; i < Points; i++)
    {
      At[i] = 0;
    }

    ak_probe_reset();
  }

  /// <summary>The engine's checkpoints, read into the managed array once the call is over.</summary>
  public static void Collect()
  {
    for (var i = 0; i < RustPoints; i++)
    {
      At[RustBase + i] = ak_probe_at((uint)i);
    }
  }

  public static void Mark(int point)
    => Volatile.Write(ref At[point],
                      Stopwatch.GetTimestamp());

  public static void MarkFirst(int point)
    => Interlocked.CompareExchange(ref At[point],
                                   Stopwatch.GetTimestamp(),
                                   0);
}
