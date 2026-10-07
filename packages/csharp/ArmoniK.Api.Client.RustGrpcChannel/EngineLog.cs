// This file is part of the ArmoniK project
//
// Copyright (C) ANEO, 2021-2026. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License")
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

using System;
using System.Collections;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

using Microsoft.Extensions.Logging;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>The engine's logs, as an <see cref="ILoggerFactory" /> receives them.</summary>
/// <remarks>
///   <para>
///     The engine calls <see cref="Trampoline" /> on its own threads, and on the caller's during the
///     runtime's creation. That call keeps the runtime callback's contract: it copies the record
///     into a queue and returns. A thread of this class writes the queue to the loggers, so that
///     a provider that blocks, or runs application code, stalls nothing of the engine.
///   </para>
///   <para>
///     The queue is bounded, and a record that finds it full is dropped and counted: a logger
///     slower than the engine loses records, and says how many, rather than holding the engine's
///     thread or the process's memory.
///   </para>
/// </remarks>
internal sealed unsafe class EngineLog
{
  /// <summary>How many records wait for the writer before the next is dropped.</summary>
  internal const int QueueCapacity = 16384;

  // The engine holds this pointer until the runtime is destroyed, and a delegate is only as alive
  // as the reference kept to it.
  private static readonly NativeMethods.LogCallback TrampolineDelegate = OnRecord;

  /// <summary>The category of what the binding itself says, as against what the engine does.</summary>
  private const string BindingCategory = "ArmoniK.Api.Client.RustGrpcChannel";

  private static EngineLog? current_;

  private EngineLog? previous_;

  private readonly BlockingCollection<Entry> queue_ = new(QueueCapacity);

  private readonly ILoggerFactory factory_;

  private readonly Dictionary<string, ILogger> loggers_ = new();

  private readonly Task writer_;

  private GCHandle self_;

  private long dropped_;

  internal EngineLog(ILoggerFactory factory)
  {
    factory_ = factory;
    self_    = GCHandle.Alloc(this);
    writer_ = Task.Factory.StartNew(Drain,
                                    CancellationToken.None,
                                    TaskCreationOptions.LongRunning,
                                    TaskScheduler.Default);
  }

  /// <summary>What a runtime created in this process logs through, while it lives.</summary>
  /// <remarks>One at a time, as the runtime is.</remarks>
  internal static EngineLog? Current
    => Volatile.Read(ref current_);

  /// <summary>The function the engine is given, as a pointer.</summary>
  internal static void* Trampoline
    => (void*)Marshal.GetFunctionPointerForDelegate(TrampolineDelegate);

  /// <summary>What the engine hands the trampoline back with each record.</summary>
  internal void* Context
    => (void*)GCHandle.ToIntPtr(self_);

  /// <summary>Makes this the log the trampoline's own catches report to.</summary>
  internal void Publish()
    => previous_ = Interlocked.Exchange(ref current_,
                                        this);

  /// <summary>Writes what is queued, and releases the context the engine was given.</summary>
  /// <remarks>Called once nothing can reach the callback any more: after the runtime is destroyed,
  /// or when its creation was refused.</remarks>
  internal void Close()
  {
    // What was current before this was published comes back: a creation the engine refused because
    // a runtime lives must not take that runtime's log away.
    Interlocked.CompareExchange(ref current_,
                                previous_,
                                this);

    queue_.CompleteAdding();
    try
    {
      writer_.Wait();
    }
    catch
    {
      // The writer catches what it writes; there is nothing else to report.
    }

    // What was dropped after the last record was written is said here, with nothing queued.
    ReportDropped();
    queue_.Dispose();

    if (self_.IsAllocated)
    {
      self_.Free();
    }
  }

  /// <summary>A failure the binding caught at the boundary where it must not propagate.</summary>
  internal void Caught(string    what,
                       Exception error)
    => Enqueue(new Entry(LogLevel.Error,
                         BindingCategory,
                         what,
                         error));

  private static void OnRecord(void*           logCtx,
                               ak_log_record* record)
  {
    try
    {
      if (GCHandle.FromIntPtr((IntPtr)logCtx)
                  .Target is EngineLog log)
      {
        log.Enqueue(Entry.Copy(record));
      }
    }
    catch
    {
      // Nothing may unwind into the engine.
    }
  }

  private void Enqueue(Entry entry)
  {
    try
    {
      if (!queue_.TryAdd(entry))
      {
        Interlocked.Increment(ref dropped_);
      }
    }
    catch
    {
      // Completed or disposed: the log is closing, and this record is after its end.
    }
  }

  private void Drain()
  {
    foreach (var entry in queue_.GetConsumingEnumerable())
    {
      ReportDropped();
      Write(entry);
    }
  }

  /// <summary>Says how many records the full queue lost since it was last said.</summary>
  private void ReportDropped()
  {
    var lost = Interlocked.Exchange(ref dropped_,
                                    0);
    if (lost > 0)
    {
      Write(new Entry(LogLevel.Warning,
                      BindingCategory,
                      $"{lost} records of the native engine's log were dropped: the logger could not keep up",
                      null));
    }
  }

  private void Write(Entry entry)
  {
    try
    {
      Log(entry);
    }
    catch
    {
      // A provider that throws costs its record and not the writer.
    }
  }

  private void Log(Entry entry)
  {
    var (category, message, fields) = entry.Decode();
    if (!loggers_.TryGetValue(category,
                              out var logger))
    {
      logger             = factory_.CreateLogger(category);
      loggers_[category] = logger;
    }

    if (!logger.IsEnabled(entry.Level))
    {
      return;
    }

    logger.Log(entry.Level,
               default,
               new State(message,
                         fields),
               entry.Error,
               static (state,
                       _) => state.ToString());
  }

  private static LogLevel Map(uint level)
    => level switch
       {
         NativeMethods.AK_LOG_ERROR => LogLevel.Error,
         NativeMethods.AK_LOG_WARN  => LogLevel.Warning,
         NativeMethods.AK_LOG_INFO  => LogLevel.Information,
         NativeMethods.AK_LOG_DEBUG => LogLevel.Debug,
         _                          => LogLevel.Trace,
       };

  /// <summary>One record on its way to a logger: the engine's, copied, or one of the binding's own.</summary>
  private sealed class Entry
  {
    private readonly byte[]? blob_;

    private readonly string? category_;

    private readonly string? message_;

    internal Entry(LogLevel  level,
                   string    category,
                   string    message,
                   Exception? error)
    {
      Level     = level;
      category_ = category;
      message_  = message;
      Error     = error;
    }

    private Entry(LogLevel level,
                  byte[]   blob)
    {
      Level = level;
      blob_ = blob;
    }

    internal LogLevel Level { get; }

    internal Exception? Error { get; }

    /// <summary>The record in one array: its target, its message, then each field's key and value,
    /// each text as a length and its bytes.</summary>
    internal static Entry Copy(ak_log_record* record)
    {
      var fieldCount = record->struct_size >= (uint)sizeof(ak_log_record)
                         ? (int)record->field_count
                         : 0;
      var size = 8 + (int)record->target.len + (int)record->message.len;
      for (var at = 0; at < fieldCount; at++)
      {
        size += 8 + (int)record->fields[at].key.len + (int)record->fields[at].value.len;
      }

      var blob = new byte[size];
      fixed (byte* start = blob)
      {
        var to = start;
        Put(ref to,
            record->target);
        Put(ref to,
            record->message);
        for (var at = 0; at < fieldCount; at++)
        {
          Put(ref to,
              record->fields[at].key);
          Put(ref to,
              record->fields[at].value);
        }
      }

      return new Entry(Map(record->level),
                       blob);
    }

    private static void Put(ref byte*       to,
                            ak_bytes_in view)
    {
      var length = (int)view.len;
      for (var shift = 0; shift < 32; shift += 8)
      {
        *to++ = (byte)(length >> shift);
      }

      if (view.len != 0)
      {
        Buffer.MemoryCopy(view.ptr,
                          to,
                          (long)view.len,
                          (long)view.len);
        to += view.len;
      }
    }

    internal (string Category, string Message, (string Key, string Value)[] Fields) Decode()
    {
      if (blob_ is null)
      {
        return (category_!, message_!, Array.Empty<(string, string)>());
      }

      fixed (byte* start = blob_)
      {
        var at = start;
        var category = Take(ref at);
        var message  = Take(ref at);
        var fields   = new List<(string, string)>();
        while (at < start + blob_.Length)
        {
          var key = Take(ref at);
          fields.Add((key, Take(ref at)));
        }

        return (category, message, fields.ToArray());
      }
    }

    private static string Take(ref byte* from)
    {
      var length = 0;
      for (var shift = 0; shift < 32; shift += 8)
      {
        length |= *from++ << shift;
      }

      var text = length == 0
                   ? string.Empty
                   : Encoding.UTF8.GetString(from,
                                             length);
      from += length;
      return text;
    }
  }

  /// <summary>What a logger is given: the text, and the fields as the state a structured provider reads.</summary>
  private sealed class State : IReadOnlyList<KeyValuePair<string, object?>>
  {
    private readonly (string Key, string Value)[] fields_;

    private readonly string message_;

    internal State(string                         message,
                   (string Key, string Value)[]   fields)
    {
      message_ = message;
      fields_  = fields;
    }

    public int Count
      => fields_.Length + 1;

    public KeyValuePair<string, object?> this[int index]
      => index < fields_.Length
           ? new KeyValuePair<string, object?>(fields_[index].Key,
                                               fields_[index].Value)
           : new KeyValuePair<string, object?>("{OriginalFormat}",
                                               message_.Replace("{",
                                                                "{{")
                                                       .Replace("}",
                                                                "}}"));

    public IEnumerator<KeyValuePair<string, object?>> GetEnumerator()
    {
      for (var at = 0; at < Count; at++)
      {
        yield return this[at];
      }
    }

    IEnumerator IEnumerable.GetEnumerator()
      => GetEnumerator();

    public override string ToString()
    {
      if (fields_.Length == 0)
      {
        return message_;
      }

      var text = new StringBuilder(message_);
      foreach (var (key, value) in fields_)
      {
        text.Append(' ')
            .Append(key)
            .Append('=')
            .Append(value);
      }

      return text.ToString();
    }
  }
}
