using System;
using System.Runtime.InteropServices;

namespace LogPumpSpike
{
    [StructLayout(LayoutKind.Sequential)]
    internal unsafe struct BytesIn
    {
        public byte* Ptr;
        public UIntPtr Len;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct LogField
    {
        public BytesIn Key;
        public BytesIn Value;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal unsafe struct LogRecord
    {
        public uint StructSize;
        public uint Level;
        public uint FieldCount;
        public uint Reserved;
        public BytesIn Target;
        public BytesIn Message;
        public LogField* Fields;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct StatsV1
    {
        public uint StructSize;
        public uint Version;
        public ulong ChannelsOpen;
        public ulong CallsStarted;
        public ulong CallsInFlight;
        public ulong CallsCompleted;
        public ulong CallsFailed;
        public ulong Dials;
        public ulong DialFailures;
        public ulong Retries;
        public ulong StreamResets;
        public ulong Goaways;
    }

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal unsafe delegate void LogCallback(IntPtr ctx, LogRecord* record);

    internal static unsafe class Native
    {
        private const string Lib = "observability_spike";

        [DllImport(Lib)] public static extern IntPtr spike_runtime_new();
        [DllImport(Lib)] public static extern void spike_runtime_free(IntPtr runtime);
        [DllImport(Lib)] public static extern int spike_set_log_callback(IntPtr runtime, LogCallback callback, IntPtr ctx, byte* filter, UIntPtr filterLen);
        [DllImport(Lib)] public static extern int spike_set_native_count(IntPtr runtime);
        [DllImport(Lib)] public static extern int spike_clear_log_callback(IntPtr runtime);
        [DllImport(Lib)] public static extern ulong spike_invoke_callback(LogCallback callback, ulong n);
        [DllImport(Lib)] public static extern ulong spike_emit(IntPtr runtime, uint threads, ulong events);
        [DllImport(Lib)] public static extern void spike_stats_bump(IntPtr runtime, ulong dials, ulong inFlight);
        [DllImport(Lib)] public static extern void spike_stats_read(IntPtr runtime, StatsV1* output);
    }
}
