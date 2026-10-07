using System;
using System.Buffers;
using System.Collections.Generic;
using System.Text;
using System.Threading;
using System.Threading.Channels;
using Microsoft.Extensions.Logging;

namespace LogPumpSpike
{
    /// <summary>How the callback hands a record to the writer thread.</summary>
    internal enum Strategy
    {
        /// <summary>Does nothing: the floor of a managed callback.</summary>
        Noop,
        /// <summary>No managed callback at all: the library's own counting callback.</summary>
        Native,
        /// <summary>Copies the record's bytes into a pooled buffer; the writer thread decodes.</summary>
        Blob,
        /// <summary>Decodes strings in the callback: what a naive binding does.</summary>
        Decode,
        /// <summary>As Blob, through a lock-free queue the writer polls: the callback never wakes a thread.</summary>
        Batch,
    }

    internal readonly struct Blob
    {
        public readonly byte[] Buffer;
        public readonly int Length;
        public Blob(byte[] buffer, int length) { Buffer = buffer; Length = length; }
    }

    internal sealed class DecodedRecord
    {
        public LogLevel Level;
        public string Target = "";
        public string Message = "";
        public KeyValuePair<string, object?>[] Fields = Array.Empty<KeyValuePair<string, object?>>();
    }

    /// <summary>
    /// The binding's side of ak_runtime_set_log_callback: the native thread copies and returns, a
    /// thread of the binding's own writes to the ILoggerFactory. The queue is bounded, and a full
    /// one drops the record and counts it, so that a slow logger never stalls the engine.
    /// </summary>
    internal sealed unsafe class LogPump : IDisposable
    {
        private readonly Strategy _strategy;
        private readonly ILoggerFactory _factory;
        private readonly int _capacity;
        private readonly Channel<Blob>? _blobs;
        private readonly Channel<DecodedRecord>? _decoded;
        private readonly Thread? _writer;
        private readonly Dictionary<string, ILogger> _loggers = new Dictionary<string, ILogger>();
        private readonly System.Collections.Concurrent.ConcurrentQueue<Blob>? _batch;
        private readonly ManualResetEventSlim _wake = new ManualResetEventSlim(false);
        private int _sinceWake;
        private int _queued;
        private long _dropped;
        private long _written;
        private int _stopping;

        // Kept alive for as long as native code may call it.
        public readonly LogCallback Callback;

        public long Dropped => Interlocked.Read(ref _dropped);
        public long Written => Interlocked.Read(ref _written);

        public LogPump(ILoggerFactory factory, Strategy strategy, int capacity)
        {
            _factory = factory;
            _strategy = strategy;
            _capacity = capacity;
            Callback = OnRecord;
            if (strategy == Strategy.Blob)
            {
                _blobs = Channel.CreateUnbounded<Blob>(new UnboundedChannelOptions { SingleReader = true });
            }
            else if (strategy == Strategy.Decode)
            {
                _decoded = Channel.CreateUnbounded<DecodedRecord>(new UnboundedChannelOptions { SingleReader = true });
            }
            else if (strategy == Strategy.Batch)
            {
                _batch = new System.Collections.Concurrent.ConcurrentQueue<Blob>();
            }
            if (strategy >= Strategy.Blob)
            {
                _writer = new Thread(WriterLoop) { IsBackground = true, Name = "armonik-log-writer" };
                _writer.Start();
            }
        }

        // Runs on the library's threads: it must not throw into native code, block, or log.
        private void OnRecord(IntPtr ctx, LogRecord* record)
        {
            try
            {
                switch (_strategy)
                {
                    case Strategy.Noop:
                        return;
                    case Strategy.Blob:
                        EnqueueBlob(record);
                        return;
                    case Strategy.Decode:
                        EnqueueDecoded(record);
                        return;
                    case Strategy.Batch:
                        EnqueueBatch(record);
                        return;
                }
            }
            catch
            {
                Interlocked.Increment(ref _dropped);
            }
        }

        private bool Admit()
        {
            if (Interlocked.Increment(ref _queued) > _capacity)
            {
                Interlocked.Decrement(ref _queued);
                Interlocked.Increment(ref _dropped);
                return false;
            }
            return true;
        }

        private static int Size(BytesIn bytes) => (int)bytes.Len.ToUInt32();

        private void EnqueueBlob(LogRecord* record)
        {
            if (!Admit())
            {
                return;
            }
            _blobs!.Writer.TryWrite(MakeBlob(record));
        }

        private void EnqueueBatch(LogRecord* record)
        {
            if (!Admit())
            {
                return;
            }
            _batch!.Enqueue(MakeBlob(record));
            // The writer polls; it is only woken early when a batch has built up.
            if (Interlocked.Increment(ref _sinceWake) == 512)
            {
                _wake.Set();
            }
        }

        private static Blob MakeBlob(LogRecord* record)
        {
            // level, field count, then length-prefixed target, message, key and value of each field.
            int total = 4 + 4 + 4 + Size(record->Target) + 4 + Size(record->Message);
            for (uint i = 0; i < record->FieldCount; i++)
            {
                total += 8 + Size(record->Fields[i].Key) + Size(record->Fields[i].Value);
            }
            byte[] buffer = ArrayPool<byte>.Shared.Rent(total);
            fixed (byte* start = buffer)
            {
                byte* cursor = start;
                *(uint*)cursor = record->Level; cursor += 4;
                *(uint*)cursor = record->FieldCount; cursor += 4;
                cursor = Put(cursor, record->Target);
                cursor = Put(cursor, record->Message);
                for (uint i = 0; i < record->FieldCount; i++)
                {
                    cursor = Put(cursor, record->Fields[i].Key);
                    cursor = Put(cursor, record->Fields[i].Value);
                }
            }
            return new Blob(buffer, total);
        }

        private static byte* Put(byte* cursor, BytesIn bytes)
        {
            int length = Size(bytes);
            *(int*)cursor = length;
            cursor += 4;
            Buffer.MemoryCopy(bytes.Ptr, cursor, length, length);
            return cursor + length;
        }

        private static string Text(BytesIn bytes) =>
            bytes.Len == UIntPtr.Zero ? "" : Encoding.UTF8.GetString(bytes.Ptr, Size(bytes));

        private void EnqueueDecoded(LogRecord* record)
        {
            if (!Admit())
            {
                return;
            }
            var decoded = new DecodedRecord
            {
                Level = Map(record->Level),
                Target = Text(record->Target),
                Message = Text(record->Message),
                Fields = new KeyValuePair<string, object?>[record->FieldCount],
            };
            for (uint i = 0; i < record->FieldCount; i++)
            {
                decoded.Fields[i] = new KeyValuePair<string, object?>(Text(record->Fields[i].Key), Text(record->Fields[i].Value));
            }
            _decoded!.Writer.TryWrite(decoded);
        }

        private static LogLevel Map(uint level) => level switch
        {
            1 => LogLevel.Error,
            2 => LogLevel.Warning,
            3 => LogLevel.Information,
            4 => LogLevel.Debug,
            _ => LogLevel.Trace,
        };

        private ILogger LoggerOf(string target)
        {
            if (!_loggers.TryGetValue(target, out ILogger? logger))
            {
                logger = _factory.CreateLogger(target);
                _loggers[target] = logger;
            }
            return logger;
        }

        private void WriterLoop()
        {
            if (_batch != null)
            {
                while (true)
                {
                    _wake.Wait(20);
                    _wake.Reset();
                    Volatile.Write(ref _sinceWake, 0);
                    bool stopping = Volatile.Read(ref _stopping) != 0;
                    while (_batch.TryDequeue(out Blob blob))
                    {
                        WriteBlob(blob);
                    }
                    if (stopping)
                    {
                        return;
                    }
                }
            }
            if (_blobs != null)
            {
                var reader = _blobs.Reader;
                while (reader.WaitToReadAsync().AsTask().GetAwaiter().GetResult())
                {
                    while (reader.TryRead(out Blob blob))
                    {
                        WriteBlob(blob);
                    }
                }
            }
            else
            {
                var reader = _decoded!.Reader;
                while (reader.WaitToReadAsync().AsTask().GetAwaiter().GetResult())
                {
                    while (reader.TryRead(out DecodedRecord? record))
                    {
                        LoggerOf(record.Target).Log(record.Level, default, new State(record.Message, record.Fields), null, State.Format);
                        Interlocked.Decrement(ref _queued);
                        Interlocked.Increment(ref _written);
                    }
                }
            }
        }

        private void WriteBlob(Blob blob)
        {
            byte[] buffer = blob.Buffer;
            int at = 0;
            LogLevel level = Map(BitConverter.ToUInt32(buffer, at)); at += 4;
            int count = BitConverter.ToInt32(buffer, at); at += 4;
            string target = Take(buffer, ref at);
            string message = Take(buffer, ref at);
            var fields = new KeyValuePair<string, object?>[count];
            for (int i = 0; i < count; i++)
            {
                string key = Take(buffer, ref at);
                string value = Take(buffer, ref at);
                fields[i] = new KeyValuePair<string, object?>(key, value);
            }
            ArrayPool<byte>.Shared.Return(buffer);
            LoggerOf(target).Log(level, default, new State(message, fields), null, State.Format);
            Interlocked.Decrement(ref _queued);
            Interlocked.Increment(ref _written);
        }

        private static string Take(byte[] buffer, ref int at)
        {
            int length = BitConverter.ToInt32(buffer, at);
            at += 4;
            string text = Encoding.UTF8.GetString(buffer, at, length);
            at += length;
            return text;
        }

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _stopping, 1) == 0)
            {
                _blobs?.Writer.TryComplete();
                _decoded?.Writer.TryComplete();
                _wake.Set();
                _writer?.Join();
            }
        }
    }

    /// <summary>The structured state an ILogger receives: the message and the engine's fields.</summary>
    internal readonly struct State : IReadOnlyList<KeyValuePair<string, object?>>
    {
        private readonly string _message;
        private readonly KeyValuePair<string, object?>[] _fields;

        public State(string message, KeyValuePair<string, object?>[] fields)
        {
            _message = message;
            _fields = fields;
        }

        public int Count => _fields.Length + 1;

        public KeyValuePair<string, object?> this[int index] =>
            index < _fields.Length ? _fields[index] : new KeyValuePair<string, object?>("{OriginalFormat}", _message);

        public IEnumerator<KeyValuePair<string, object?>> GetEnumerator()
        {
            for (int i = 0; i < Count; i++)
            {
                yield return this[i];
            }
        }

        System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => GetEnumerator();

        public static readonly Func<State, Exception?, string> Format = (state, _) => state._message;
    }
}
