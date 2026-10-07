using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Diagnostics.Metrics;
using System.Text;
using System.Threading;
using Microsoft.Extensions.Logging;

namespace LogPumpSpike
{
    internal sealed class CountingLoggerFactory : ILoggerFactory
    {
        public long Count;
        public long Chars;
        public void AddProvider(ILoggerProvider provider) { }
        public ILogger CreateLogger(string categoryName) => new CountingLogger(this);
        public void Dispose() { }

        private sealed class CountingLogger : ILogger
        {
            private readonly CountingLoggerFactory _owner;
            public CountingLogger(CountingLoggerFactory owner) { _owner = owner; }
            public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;
            public bool IsEnabled(LogLevel logLevel) => true;
            public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
            {
                // What a console or file provider does at least: render the message.
                string text = formatter(state, exception);
                Interlocked.Increment(ref _owner.Count);
                Interlocked.Add(ref _owner.Chars, text.Length);
            }
        }
    }

    internal static unsafe class Program
    {
        private static int Main(string[] args)
        {
            string mode = args.Length > 0 ? args[0] : "log";
            return mode == "log" ? LogMode(args) : mode == "cross" ? CrossMode(args) : MetricsMode();
        }

        // The crossing and the callback alone, with no event rendered: native calls the managed
        // callback in a tight loop and reports the time.
        private static int CrossMode(string[] args)
        {
            var strategy = (Strategy)Enum.Parse(typeof(Strategy), args[1], true);
            ulong n = ulong.Parse(args[2]);
            var factory = new CountingLoggerFactory();
            using var pump = new LogPump(factory, strategy, 50_000_000);
            Native.spike_invoke_callback(pump.Callback, 20_000);
            var samples = new List<double>();
            long sent = 20_000;
            for (int round = 0; round < 15; round++)
            {
                samples.Add((double)Native.spike_invoke_callback(pump.Callback, n) / n);
                sent += (long)n;
                // Let the writer catch up, so that each round starts with an empty queue.
                while (strategy >= Strategy.Blob && pump.Written + pump.Dropped < sent)
                {
                    Thread.Sleep(1);
                }
            }
            samples.Sort();
            var drain = Stopwatch.StartNew();
            Console.WriteLine(
                $"{RuntimeName()} cross {strategy,-6} n={n} | ns per callback: min {samples[0]:F0} median {samples[7]:F0} max {samples[14]:F0} | written {pump.Written} dropped {pump.Dropped}");
            _ = drain;
            return 0;
        }

        private static int LogMode(string[] args)
        {
            var strategy = (Strategy)Enum.Parse(typeof(Strategy), args[1], true);
            ulong events = ulong.Parse(args[2]);
            uint threads = uint.Parse(args[3]);
            int capacity = args.Length > 4 ? int.Parse(args[4]) : 1_000_000;

            IntPtr runtime = Native.spike_runtime_new();
            var factory = new CountingLoggerFactory();
            using var pump = new LogPump(factory, strategy, capacity);
            byte[] filter = new byte[1];
            fixed (byte* f = filter)
            {
                // An empty filter keeps the default: the engine at info.
                int status = strategy == Strategy.Native
                    ? Native.spike_set_native_count(runtime)
                    : Native.spike_set_log_callback(runtime, pump.Callback, IntPtr.Zero, f, UIntPtr.Zero);
                if (status != 0) { Console.WriteLine($"registration refused: {status}"); return 1; }
            }

            // Warm the JIT and the delegate thunk.
            Native.spike_emit(runtime, 1, 20_000);
            Thread.Sleep(200);

            var samples = new List<double>();
            for (int round = 0; round < 5; round++)
            {
                ulong nanos = Native.spike_emit(runtime, threads, events);
                samples.Add((double)nanos / (events * threads));
            }
            samples.Sort();
            long emitted = (long)(events * threads * 5 + 20_000);
            var drain = Stopwatch.StartNew();
            while (strategy >= Strategy.Blob && pump.Written + pump.Dropped < emitted && drain.ElapsedMilliseconds < 20_000)
            {
                Thread.Sleep(5);
            }
            Console.WriteLine(
                $"{RuntimeName()} {strategy,-6} threads={threads} events/thread={events} | native-side ns/event (callback included): min {samples[0]:F0} median {samples[2]:F0} max {samples[4]:F0} | written {pump.Written} dropped {pump.Dropped} of {emitted} | drained in {drain.ElapsedMilliseconds} ms");
            Native.spike_clear_log_callback(runtime);
            return 0;
        }

        private static string RuntimeName() =>
            System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription;

        private static int MetricsMode()
        {
            IntPtr runtime = Native.spike_runtime_new();
            Native.spike_stats_bump(runtime, 3, 2);

            // One read per collection, however many instruments ask: the instruments share a
            // snapshot taken at most every 50 ms.
            long reads = 0;
            StatsV1 snapshot = default;
            long taken = 0;
            StatsV1 Read()
            {
                long now = Stopwatch.GetTimestamp();
                if (taken == 0 || (now - taken) * 1000 / Stopwatch.Frequency >= 50)
                {
                    StatsV1 fresh = default;
                    fresh.StructSize = (uint)sizeof(StatsV1);
                    Native.spike_stats_read(runtime, &fresh);
                    snapshot = fresh;
                    taken = now;
                    reads++;
                }
                return snapshot;
            }

            using var meter = new Meter("ArmoniK.Api.Client.RustGrpcChannel", "1.0");
            meter.CreateObservableCounter("armonik.engine.dials", () => (long)Read().Dials, description: "Dials started");
            meter.CreateObservableCounter("armonik.engine.retries", () => (long)Read().Retries, description: "Retries");
            meter.CreateObservableGauge("armonik.engine.calls_in_flight", () => (long)Read().CallsInFlight, description: "Calls in flight");
            meter.CreateObservableGauge("armonik.engine.channels_open", () => (long)Read().ChannelsOpen, description: "Channels open");

            var seen = new Dictionary<string, long>();
            using var listener = new MeterListener();
            listener.InstrumentPublished = (instrument, l) =>
            {
                if (instrument.Meter.Name == meter.Name) { l.EnableMeasurementEvents(instrument); }
            };
            listener.SetMeasurementEventCallback<long>((instrument, value, tags, state) => seen[instrument.Name] = value);
            listener.Start();
            listener.RecordObservableInstruments();
            foreach (var pair in seen) { Console.WriteLine($"  {pair.Key} = {pair.Value}"); }

            // What a collection costs.
            var watch = Stopwatch.StartNew();
            const int Rounds = 20_000;
            for (int i = 0; i < Rounds; i++) { listener.RecordObservableInstruments(); }
            watch.Stop();
            Console.WriteLine($"{RuntimeName()} metrics: {watch.Elapsed.TotalMilliseconds * 1000 / Rounds:F2} us per collection of 4 instruments, {reads} native reads in {Rounds} collections");
            return 0;
        }
    }
}
