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
using System.Collections.Generic;
using System.Diagnostics.Metrics;
using System.Linq;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>What the native engine counts, as the observable instruments of five meters.</summary>
///
/// A meter per group, under <see cref="Prefix" />: Calls, Throttle, Connections, Bytes and Host.
/// Every instrument is read when a collector collects, one read for each instrument, so a process
/// that listens to none makes no call into the engine. What one instrument reports is read at its
/// own instant, not at the one of another. The instruments of an endpoint, which are all but the
/// memory ceiling's and the dropped logs, read <c>ak_channel_stats</c> through one open channel of
/// each endpoint and carry its <c>server.address</c> and <c>server.port</c>; those two read
/// <c>ak_runtime_stats</c>, which no endpoint owns. A library built without its counters says so
/// in the first answer, and then nothing of the engine's is registered. A host filters with
/// <c>AddMeter</c>, with views or with a listener; a tag is a status, a reason, an origin or a
/// server, and never a method, whose values are unbounded.
internal sealed class EngineMetrics : IDisposable
{
  /// <summary>The name every meter starts with.</summary>
  internal const string Prefix = "ArmoniK.Api.Client.RustGrpcChannel";

  private const string StatusTag = "rpc.response.status_code";

  private const string OriginTag = "armonik.retry.origin";

  private const string ReasonTag = "armonik.retry.reason";

  private const string OutcomeTag = "armonik.dial.outcome";

  private const string CloseTag = "armonik.connection.close_reason";

  private const string ResetTag = "http2.reset.reason";

  // OpenTelemetry's attributes for the server a client talks to.
  private const string AddressTag = "server.address";

  private const string PortTag = "server.port";

  /// <summary>The first four fields of the record: all a host needs to learn whether the library counts.</summary>
  private const uint HeadSize = 16;

  private static readonly string[] Statuses =
  {
    "OK",
    "CANCELLED",
    "UNKNOWN",
    "INVALID_ARGUMENT",
    "DEADLINE_EXCEEDED",
    "NOT_FOUND",
    "ALREADY_EXISTS",
    "PERMISSION_DENIED",
    "RESOURCE_EXHAUSTED",
    "FAILED_PRECONDITION",
    "ABORTED",
    "OUT_OF_RANGE",
    "UNIMPLEMENTED",
    "INTERNAL",
    "UNAVAILABLE",
    "DATA_LOSS",
    "UNAUTHENTICATED",
  };

  /// <summary>The HTTP/2 error codes of RFC 9113, and the slot for any other.</summary>
  private static readonly string[] Resets =
  {
    "NO_ERROR",
    "PROTOCOL_ERROR",
    "INTERNAL_ERROR",
    "FLOW_CONTROL_ERROR",
    "SETTINGS_TIMEOUT",
    "STREAM_CLOSED",
    "FRAME_SIZE_ERROR",
    "REFUSED_STREAM",
    "CANCEL",
    "COMPRESSION_ERROR",
    "CONNECT_ERROR",
    "ENHANCE_YOUR_CALM",
    "INADEQUATE_SECURITY",
    "HTTP_1_1_REQUIRED",
    "other",
  };

  private static readonly string[] HttpStatuses =
  {
    "408",
    "429",
    "500",
    "502",
    "503",
    "504",
    "other",
  };

  private static readonly string[] CloseReasons =
  {
    "goaway",
    "keepalive_timeout",
    "idle_timeout",
    "io_error",
    "local_close",
    "peer_closed",
    "protocol_error",
    "other",
  };

  private readonly ulong runtime_;

  /// <summary>The channels the runtime has open, by handle and by the endpoint the engine names them with.</summary>
  private readonly Func<IReadOnlyList<(ulong Handle, string Endpoint)>> channels_;

  private readonly List<Meter> meters_ = new();

  private EngineMetrics(ulong                                              runtime,
                        Func<IReadOnlyList<(ulong Handle, string Endpoint)>> channels,
                        bool                                               counting,
                        EngineLog?                                         log)
  {
    runtime_  = runtime;
    channels_ = channels;
    var version = typeof(EngineMetrics).Assembly.GetName()
                                       .Version?.ToString();

    if (counting)
    {
      Calls(Make("Calls",
                 version));
      Throttle(Make("Throttle",
                    version));
      Connections(Make("Connections",
                       version));
      Bytes(Make("Bytes",
                 version));
    }

    var host = Make("Host",
                    version);
    if (counting)
    {
      Host(host);
    }

    if (log is not null)
    {
      host.CreateObservableCounter("armonik.client.logs.dropped",
                                   () => log.Dropped,
                                   "{record}",
                                   "Log records that the bounded queue of the binding dropped because the logger could not keep up");
    }
  }

  /// <summary>The instruments of a runtime's engine and log, or none when the library counts nothing and no log is given.</summary>
  /// <param name="runtime">The runtime's handle.</param>
  /// <param name="log">The log of the runtime, whose drops are counted, or none.</param>
  /// <param name="channels">The runtime's open channels, which the counters of each endpoint are read through.</param>
  internal static EngineMetrics? TryCreate(ulong                                              runtime,
                                           Func<IReadOnlyList<(ulong Handle, string Endpoint)>> channels,
                                           EngineLog?                                         log)
  {
    var counting = Counts(runtime);
    return counting || log is not null
             ? new EngineMetrics(runtime,
                                 channels,
                                 counting,
                                 log)
             : null;
  }

  /// <inheritdoc />
  public void Dispose()
  {
    foreach (var meter in meters_)
    {
      meter.Dispose();
    }

    meters_.Clear();
  }

  private static unsafe bool Counts(ulong runtime)
  {
    try
    {
      var head = new ak_stats
                 {
                   struct_size = HeadSize,
                 };
      return NativeMethods.ak_runtime_stats(runtime,
                                            &head,
                                            null) == ak_status.AK_STATUS_OK && (head.flags & NativeMethods.AK_STATS_COUNTING) != 0;
    }
    catch (Exception error) when (error is EntryPointNotFoundException || error is DllNotFoundException)
    {
      // The library has no ak_runtime_stats: it counts nothing.
      return false;
    }
  }

  private Meter Make(string      group,
                     string?     version)
  {
    var meter = new Meter($"{Prefix}.{group}",
                          version);
    meters_.Add(meter);
    return meter;
  }

  /// <summary>The engine's runtime-wide counters now, or none once the runtime has gone.</summary>
  private unsafe Snapshot? Read()
  {
    var raw = new ak_stats
              {
                struct_size = (uint)sizeof(ak_stats),
              };
    return NativeMethods.ak_runtime_stats(runtime_,
                                          &raw,
                                          null) == ak_status.AK_STATUS_OK
             ? Snapshot.Of(raw)
             : null;
  }

  /// <summary>What the engine counted for each endpoint a channel is open on, read through one open channel of each.</summary>
  /// <remarks>Two channels to one endpoint count into one registry, so they read the same and the
  /// endpoint is read once. A channel released while it is read is skipped for the next channel of
  /// its endpoint, if there is one.</remarks>
  private unsafe IReadOnlyList<Sample> Endpoints()
  {
    var samples = new List<Sample>();
    var seen    = new HashSet<string>(StringComparer.Ordinal);
    foreach (var (handle, endpoint) in channels_())
    {
      if (seen.Contains(endpoint))
      {
        continue;
      }

      var raw = new ak_stats
                {
                  struct_size = (uint)sizeof(ak_stats),
                };
      if (NativeMethods.ak_channel_stats(handle,
                                         &raw,
                                         null) != ak_status.AK_STATUS_OK)
      {
        continue;
      }

      seen.Add(endpoint);
      samples.Add(new Sample(ServerTags(endpoint),
                             Snapshot.Of(raw)));
    }

    return samples;
  }

  /// <summary>The tags that name an endpoint: its host, and its port when it states one.</summary>
  /// <param name="endpoint">The endpoint as the engine names it: host and port, and nothing else.</param>
  internal static KeyValuePair<string, object?>[] ServerTags(string endpoint)
  {
    var start = endpoint.IndexOf("://",
                                 StringComparison.Ordinal);
    var authority = start < 0
                      ? endpoint
                      : endpoint.Substring(start + 3);
    string  host;
    string? port = null;
    var     close = authority.IndexOf(']');
    if (authority.StartsWith("[",
                             StringComparison.Ordinal) && close > 0)
    {
      host = authority.Substring(1,
                                 close - 1);
      if (authority.Length > close + 2 && authority[close + 1] == ':')
      {
        port = authority.Substring(close + 2);
      }
    }
    else
    {
      var colon = authority.LastIndexOf(':');
      host = colon < 0
               ? authority
               : authority.Substring(0,
                                     colon);
      port = colon < 0
               ? null
               : authority.Substring(colon + 1);
    }

    return port is not null && int.TryParse(port,
                                            out var number)
             ? new[]
               {
                 new KeyValuePair<string, object?>(AddressTag,
                                                   host),
                 new KeyValuePair<string, object?>(PortTag,
                                                   number),
               }
             : new[]
               {
                 new KeyValuePair<string, object?>(AddressTag,
                                                   host),
               };
  }

  /// <summary>Each endpoint's measurements, with the endpoint's tags added to what <paramref name="measure" /> gives.</summary>
  private IEnumerable<Measurement<T>> Over<T>(Func<Snapshot, IEnumerable<Measurement<T>>> measure)
    where T : struct
  {
    foreach (var sample in Endpoints())
    {
      foreach (var measurement in measure(sample.Stats))
      {
        yield return Tagged(measurement,
                            sample.Tags);
      }
    }
  }

  private static Measurement<T> Tagged<T>(Measurement<T>                  measurement,
                                          KeyValuePair<string, object?>[] server)
    where T : struct
    => new(measurement.Value,
           measurement.Tags.ToArray()
                      .Concat(server)
                      .ToArray());

  private static IEnumerable<Measurement<long>> One(long value)
    => new[]
       {
         new Measurement<long>(value),
       };

  /// <summary>One value for each endpoint.</summary>
  private IEnumerable<Measurement<long>> Of(Func<Snapshot, long> pick)
    => Over(snapshot => One(pick(snapshot)));

  /// <summary>One value for the runtime, which no endpoint owns.</summary>
  private IEnumerable<Measurement<long>> OfRuntime(Func<Snapshot, long> pick)
    => Read() is { } snapshot
         ? One(pick(snapshot))
         : Enumerable.Empty<Measurement<long>>();

  /// <summary>The slots of an array that hold something, each as a series of its tag's value.</summary>
  private IEnumerable<Measurement<long>> Slots(Func<Snapshot, IReadOnlyList<long>> pick,
                                               string                              tag,
                                               Func<int, string>                   name)
    => Over(snapshot => SlotsOf(pick(snapshot),
                                tag,
                                name));

  private static IEnumerable<Measurement<long>> SlotsOf(IReadOnlyList<long> slots,
                                                        string              tag,
                                                        Func<int, string>   name)
  {
    for (var slot = 0; slot < slots.Count; slot++)
    {
      if (slots[slot] != 0)
      {
        yield return new Measurement<long>(slots[slot],
                                           new KeyValuePair<string, object?>(tag,
                                                                             name(slot)));
      }
    }
  }

  private void Counter(Meter                meter,
                       string               name,
                       string               unit,
                       string               description,
                       Func<Snapshot, long> pick)
    => meter.CreateObservableCounter(name,
                                     () => Of(pick),
                                     unit,
                                     description);

  private void Gauge(Meter                meter,
                     string               name,
                     string               unit,
                     string               description,
                     Func<Snapshot, long> pick)
    => meter.CreateObservableGauge(name,
                                   () => Of(pick),
                                   unit,
                                   description);

  private void Calls(Meter meter)
  {
    Counter(meter,
            "armonik.client.calls.started",
            "{call}",
            "Calls started",
            snapshot => snapshot.Started);
    meter.CreateObservableUpDownCounter("armonik.client.calls.active",
                                        () => Of(snapshot => Math.Max(0,
                                                                      snapshot.Started - snapshot.Ended.Sum())),
                                        "{call}",
                                        "Calls started and not ended");
    Counter(meter,
            "armonik.client.calls.failed",
            "{call}",
            "Calls that ended with a status other than OK",
            snapshot => snapshot.Ended.Sum() - snapshot.Ended[0]);
    Counter(meter,
            "armonik.client.calls.deadline_exceeded",
            "{call}",
            "Calls that ended DEADLINE_EXCEEDED",
            snapshot => snapshot.Ended[4]);
    Counter(meter,
            "armonik.client.calls.unimplemented",
            "{call}",
            "Calls that ended UNIMPLEMENTED",
            snapshot => snapshot.Ended[12]);
    Counter(meter,
            "armonik.client.messages.sent",
            "{message}",
            "Messages sent, each once whatever the attempts that send it",
            snapshot => snapshot.MessagesSent);
    Counter(meter,
            "armonik.client.messages.received",
            "{message}",
            "Messages received",
            snapshot => snapshot.MessagesReceived);
    meter.CreateObservableCounter("armonik.client.calls.ended",
                                  () => Slots(snapshot => snapshot.Ended,
                                              StatusTag,
                                              slot => Statuses[slot]),
                                  "{call}",
                                  "Calls ended, by the status they ended with");
    meter.CreateObservableCounter("armonik.client.retries",
                                  Retries,
                                  "{retry}",
                                  "Calls sent again after they failed, by the failure that was retried");
    Counter(meter,
            "armonik.client.calls.not_replayable",
            "{call}",
            "Calls whose messages outgrew a replay ceiling, so that they are never tried again",
            snapshot => snapshot.NotReplayable);
    Counter(meter,
            "armonik.client.requests.resent",
            "{request}",
            "Requests the peer never processed, sent again at once",
            snapshot => snapshot.Resends);
  }

  private IEnumerable<Measurement<long>> Retries()
    => Over(RetriesOf);

  private static IEnumerable<Measurement<long>> RetriesOf(Snapshot snapshot)
  {
    for (var slot = 0; slot < snapshot.Retries.Length; slot++)
    {
      if (snapshot.Retries[slot] == 0)
      {
        continue;
      }

      var (origin, reason) = RetrySlot(slot);
      yield return reason is null
                     ? new Measurement<long>(snapshot.Retries[slot],
                                             new KeyValuePair<string, object?>(OriginTag,
                                                                               origin))
                     : new Measurement<long>(snapshot.Retries[slot],
                                             new KeyValuePair<string, object?>(OriginTag,
                                                                               origin),
                                             new KeyValuePair<string, object?>(ReasonTag,
                                                                               reason));
    }
  }

  /// <summary>What a slot of the retries array names, as the header lays them out.</summary>
  internal static (string Origin, string? Reason) RetrySlot(int slot)
    => slot switch
       {
         < 16 => ("status", Statuses[slot + 1]),
         < 23 => ("http", HttpStatuses[slot - 16]),
         < 38 => ("reset", Resets[slot - 23]),
         38   => ("pushback", null),
         39   => ("dial", null),
         _    => ("connection", null),
       };

  private void Throttle(Meter meter)
  {
    Counter(meter,
            "armonik.client.throttle.retries_refused",
            "{retry}",
            "Retries the adaptive estimate of a channel stopped",
            snapshot => snapshot.RetriesRefused);
    meter.CreateObservableGauge("armonik.client.throttle.cap",
                                () => Over(snapshot => snapshot.Capped > 0
                                                         ? new[]
                                                           {
                                                             new Measurement<double>(snapshot.CapPerSecond),
                                                           }
                                                         : Enumerable.Empty<Measurement<double>>()),
                                "{call}/s",
                                "The rate of first attempts the capped channels of an endpoint allow together; absent while none is capped");
    Gauge(meter,
          "armonik.client.throttle.channels_capped",
          "{channel}",
          "Channels of an endpoint whose first attempts are capped",
          snapshot => snapshot.Capped);
    Gauge(meter,
          "armonik.client.throttle.channels_retries_closed",
          "{channel}",
          "Channels of an endpoint whose estimate has stopped retries",
          snapshot => snapshot.RetriesClosed);
    Gauge(meter,
          "armonik.client.throttle.calls_waiting",
          "{call}",
          "Calls waiting for their turn at a cap",
          snapshot => snapshot.WaitingAtCap);
  }

  private void Connections(Meter meter)
  {
    meter.CreateObservableCounter("armonik.client.dials",
                                  () => Over(snapshot => new[]
                                                         {
                                                           new Measurement<long>(snapshot.DialsSucceeded,
                                                                                 new KeyValuePair<string, object?>(OutcomeTag,
                                                                                                                   "succeeded")),
                                                           new Measurement<long>(snapshot.DialsFailed,
                                                                                 new KeyValuePair<string, object?>(OutcomeTag,
                                                                                                                   "failed")),
                                                         }),
                                  "{dial}",
                                  "Connections the channels tried to open, by their outcome");
    meter.CreateObservableUpDownCounter("armonik.client.dials.pending",
                                        () => Of(snapshot => Math.Max(0,
                                                                      snapshot.DialsTried - snapshot.DialsSucceeded - snapshot.DialsFailed)),
                                        "{dial}",
                                        "Dials in flight");
    meter.CreateObservableUpDownCounter("armonik.client.connections.open",
                                        () => Of(snapshot => Math.Max(0,
                                                                      snapshot.DialsSucceeded - snapshot.Closed.Sum())),
                                        "{connection}",
                                        "HTTP/2 connections open");
    meter.CreateObservableCounter("armonik.client.connections.closed",
                                  () => Slots(snapshot => snapshot.Closed,
                                              CloseTag,
                                              slot => CloseReasons[slot]),
                                  "{connection}",
                                  "HTTP/2 connections that ended, by the reason");
    meter.CreateObservableCounter("armonik.client.streams.reset",
                                  () => Slots(snapshot => snapshot.Reset,
                                              ResetTag,
                                              slot => Resets[slot]),
                                  "{stream}",
                                  "Streams the peer reset, by the HTTP/2 error code");
    Gauge(meter,
          "armonik.client.streams.calls_waiting",
          "{call}",
          "Calls waiting for a connection to open or to have room",
          snapshot => snapshot.WaitingForStream);
  }

  private void Bytes(Meter meter)
  {
    Counter(meter,
            "armonik.client.wire.sent",
            "By",
            "HTTP/2 bytes written to the connections, above TLS",
            snapshot => snapshot.WireSent);
    Counter(meter,
            "armonik.client.wire.received",
            "By",
            "HTTP/2 bytes read from the connections, above TLS",
            snapshot => snapshot.WireReceived);
    Counter(meter,
            "armonik.client.compression.input",
            "By",
            "Message bytes as the callers wrote them, before compression",
            snapshot => snapshot.Raw);
    Counter(meter,
            "armonik.client.compression.output",
            "By",
            "Message bytes as the engine sent them, after compression",
            snapshot => snapshot.Sent);
    meter.CreateObservableGauge("armonik.client.compression.gain",
                                () => Over(snapshot => snapshot.Raw > 0
                                                         ? new[]
                                                           {
                                                             new Measurement<double>(1.0 - (double)snapshot.Sent / snapshot.Raw),
                                                           }
                                                         : Enumerable.Empty<Measurement<double>>()),
                                "1",
                                "One minus the ratio of the message bytes sent to the message bytes before compression; absent while nothing was sent");
  }

  /// <summary>The window waits are an endpoint's; the memory ceiling and the log are the runtime's, which no endpoint owns.</summary>
  private void Host(Meter meter)
  {
    Counter(meter,
            "armonik.client.host.window.waits",
            "{wait}",
            "Deliveries that found every credit of the receive window spent",
            snapshot => snapshot.WindowWaits);
    meter.CreateObservableCounter("armonik.client.host.memory.waits",
                                  () => OfRuntime(snapshot => snapshot.MemoryWaits),
                                  "{wait}",
                                  "Reads held back and sends made to wait by the memory ceiling");
    meter.CreateObservableCounter("armonik.client.host.memory.refusals",
                                  () => OfRuntime(snapshot => snapshot.MemoryRefusals),
                                  "{refusal}",
                                  "Sends refused and received messages dropped by the memory ceiling");
  }

  /// <summary>An endpoint's tags, and what the engine counted for it at one read.</summary>
  private sealed class Sample
  {
    internal Sample(KeyValuePair<string, object?>[] tags,
                    Snapshot                        stats)
    {
      Tags  = tags;
      Stats = stats;
    }

    internal KeyValuePair<string, object?>[] Tags { get; }

    internal Snapshot Stats { get; }
  }

  /// <summary>The engine's counters at one read, as numbers a meter reports.</summary>
  private sealed class Snapshot
  {
    internal long Started;

    internal long[] Ended = new long[17];

    internal long MessagesSent;

    internal long MessagesReceived;

    internal long[] Retries = new long[41];

    internal long RetriesRefused;

    internal long NotReplayable;

    internal long Resends;

    internal long DialsTried;

    internal long DialsSucceeded;

    internal long DialsFailed;

    internal long[] Closed = new long[8];

    internal long[] Reset = new long[15];

    internal long WireSent;

    internal long WireReceived;

    internal long Raw;

    internal long Sent;

    internal long WindowWaits;

    internal long MemoryWaits;

    internal long MemoryRefusals;

    internal double CapPerSecond;

    internal long Capped;

    internal long RetriesClosed;

    internal long WaitingAtCap;

    internal long WaitingForStream;

    internal static unsafe Snapshot Of(ak_stats raw)
    {
      var snapshot = new Snapshot
                     {
                       Started          = (long)raw.calls_started,
                       MessagesSent     = (long)raw.messages_sent,
                       MessagesReceived = (long)raw.messages_received,
                       RetriesRefused   = (long)raw.retries_refused,
                       NotReplayable    = (long)raw.calls_not_replayable,
                       Resends          = (long)raw.resends,
                       DialsTried       = (long)raw.dials_tried,
                       DialsSucceeded   = (long)raw.dials_succeeded,
                       DialsFailed      = (long)raw.dials_failed,
                       WireSent         = (long)raw.wire_bytes_sent,
                       WireReceived     = (long)raw.wire_bytes_received,
                       Raw              = (long)raw.message_bytes_raw,
                       Sent             = (long)raw.message_bytes_sent,
                       WindowWaits      = (long)raw.host_window_waits,
                       MemoryWaits      = (long)raw.host_memory_waits,
                       MemoryRefusals   = (long)raw.host_memory_refusals,
                       CapPerSecond     = raw.throttle_cap_per_second,
                       Capped           = (long)raw.channels_capped,
                       RetriesClosed    = (long)raw.channels_retries_closed,
                       WaitingAtCap     = (long)raw.calls_waiting_at_cap,
                       WaitingForStream = (long)raw.calls_waiting_for_stream,
                     };
      Copy(raw.calls_ended,
           snapshot.Ended);
      Copy(raw.retries,
           snapshot.Retries);
      Copy(raw.connections_closed,
           snapshot.Closed);
      Copy(raw.streams_reset,
           snapshot.Reset);
      return snapshot;
    }

    private static unsafe void Copy(ulong* from,
                                    long[] to)
    {
      for (var slot = 0; slot < to.Length; slot++)
      {
        to[slot] = (long)from[slot];
      }
    }
  }
}
