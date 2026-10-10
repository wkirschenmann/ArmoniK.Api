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
using System.Threading.Tasks;

using Grpc.Core;

using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What a collector sees of the engine's counters, through the meters of the binding.</summary>
///
/// The suites run once for each build of the engine: against the build with its counters, the
/// instruments hold what the calls did; against the other, none is registered.
[TestFixture]
public class EngineMetricsTests : EchoServerFixture
{
  private static bool Counts
    => NativeEngineSelection.Wanted == NativeEngineBuild.Metrics;

  /// <summary>A listener on the binding's meters, which reads every instrument when it is asked to.</summary>
  private sealed class Collector : IDisposable
  {
    private readonly MeterListener listener_ = new();

    internal List<string> Instruments { get; } = new();

    internal List<(string Instrument, double Value, Dictionary<string, object?> Tags)> Measurements { get; } = new();

    internal Collector()
    {
      listener_.InstrumentPublished = (instrument,
                                       listener) =>
                                      {
                                        if (instrument.Meter.Name.StartsWith(EngineMetrics.Prefix,
                                                                             StringComparison.Ordinal))
                                        {
                                          Instruments.Add(instrument.Name);
                                          listener.EnableMeasurementEvents(instrument);
                                        }
                                      };
      listener_.SetMeasurementEventCallback<long>((instrument,
                                                   value,
                                                   tags,
                                                   _) => Measurements.Add((instrument.Name, value, ToMap(tags))));
      listener_.SetMeasurementEventCallback<double>((instrument,
                                                     value,
                                                     tags,
                                                     _) => Measurements.Add((instrument.Name, value, ToMap(tags))));
      listener_.Start();
    }

    private static Dictionary<string, object?> ToMap(ReadOnlySpan<KeyValuePair<string, object?>> tags)
    {
      var map = new Dictionary<string, object?>();
      foreach (var tag in tags)
      {
        map[tag.Key] = tag.Value;
      }

      return map;
    }

    /// <summary>Reads every instrument now, and answers what they reported.</summary>
    internal void Collect()
    {
      Measurements.Clear();
      listener_.RecordObservableInstruments();
    }

    internal double Value(string instrument,
                          params (string Tag, string Value)[] tags)
      => Measurements.Where(measurement => measurement.Instrument == instrument && tags.All(tag => measurement.Tags.TryGetValue(tag.Tag,
                                                                                                                              out var value) && Equals(value?.ToString(),
                                                                                                                                                       tag.Value)))
                     .Sum(measurement => measurement.Value);

    internal bool Reports(string instrument)
      => Measurements.Any(measurement => measurement.Instrument == instrument);

    public void Dispose()
      => listener_.Dispose();
  }

  private async Task Say(NativeChannel channel,
                         int           times)
  {
    for (var call = 0; call < times; call++)
    {
      var reply = await Client(channel)
                        .SayAsync(new EchoRequest
                                  {
                                    Text = "counted",
                                  })
                        .ResponseAsync.ConfigureAwait(false);
      Assert.That(reply.Text,
                  Is.EqualTo("counted"));
    }
  }

  [Test]
  public void AnEngineThatCountsNothingRegistersNoInstrument()
  {
    if (Counts)
    {
      Assert.Ignore("this run is against the build with its counters");
    }

    using var collector = new Collector();

    Assert.That(collector.Instruments,
                Is.Empty);
  }

  [Test]
  public void AnEngineThatCountsRegistersTheInstrumentsOfFiveMeters()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    using var collector = new Collector();

    Assert.That(collector.Instruments,
                Is.SupersetOf(new[]
                              {
                                "armonik.client.calls.started",
                                "armonik.client.calls.active",
                                "armonik.client.calls.failed",
                                "armonik.client.calls.deadline_exceeded",
                                "armonik.client.calls.unimplemented",
                                "armonik.client.messages.sent",
                                "armonik.client.messages.received",
                                "armonik.client.retries",
                                "armonik.client.throttle.cap",
                                "armonik.client.connections.open",
                                "armonik.client.wire.sent",
                                "armonik.client.compression.gain",
                                "armonik.client.host.window.waits",
                              }));
  }

  [Test]
  public async Task TheInstrumentsHoldWhatTheCallsDid()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    using var collector = new Collector();
    await using var channel = Runtime.Channel(Endpoint);
    collector.Collect();
    var before = (collector.Reports("armonik.client.compression.gain"), collector.Reports("armonik.client.throttle.cap"));

    await Say(channel,
              3)
      .ConfigureAwait(false);
    collector.Collect();

    Assert.Multiple(() =>
                    {
                      Assert.That(before,
                                  Is.EqualTo((false, false)),
                                  "no gain while nothing was sent, no cap while none is capped");
                      Assert.That(collector.Reports("armonik.client.throttle.cap"),
                                  Is.False);
                      Assert.That(collector.Value("armonik.client.compression.gain"),
                                  Is.EqualTo(0).Within(1e-9),
                                  "nothing compressed, nothing gained");
                      Assert.That(collector.Value("armonik.client.calls.started"),
                                  Is.EqualTo(3));
                      Assert.That(collector.Value("armonik.client.calls.ended",
                                                  ("rpc.response.status_code", "OK")),
                                  Is.EqualTo(3));
                      Assert.That(collector.Value("armonik.client.calls.active"),
                                  Is.EqualTo(0));
                      Assert.That(collector.Value("armonik.client.calls.failed"),
                                  Is.EqualTo(0));
                      Assert.That(collector.Value("armonik.client.messages.sent"),
                                  Is.EqualTo(3));
                      Assert.That(collector.Value("armonik.client.messages.received"),
                                  Is.EqualTo(3));
                      Assert.That(collector.Value("armonik.client.wire.sent"),
                                  Is.GreaterThan(0));
                      Assert.That(collector.Value("armonik.client.wire.received"),
                                  Is.GreaterThan(0));
                      Assert.That(collector.Value("armonik.client.dials",
                                                  ("armonik.dial.outcome", "succeeded")),
                                  Is.EqualTo(1));
                      Assert.That(collector.Value("armonik.client.connections.open"),
                                  Is.EqualTo(1));
                      Assert.That(collector.Value("armonik.client.dials.pending"),
                                  Is.EqualTo(0));
                    });
  }

  [Test]
  public async Task TwoChannelsToTwoEndpointsAreTwoSeriesTaggedWithTheirServer()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    using var other = EchoServerProcess.Start();
    var       one   = new Uri(Endpoint);
    var       two   = new Uri(other.Endpoint);
    using var collector = new Collector();
    await using var first  = Runtime.Channel(Endpoint);
    await using var second = Runtime.Channel(other.Endpoint);
    await Say(first,
              2)
      .ConfigureAwait(false);
    await Say(second,
              3)
      .ConfigureAwait(false);
    collector.Collect();

    var ofOne = new[]
                {
                  ("server.address", one.Host),
                  ("server.port", one.Port.ToString()),
                };
    var ofTwo = new[]
                {
                  ("server.address", two.Host),
                  ("server.port", two.Port.ToString()),
                };
    var perEndpoint = collector.Measurements.Where(measurement => measurement.Instrument == "armonik.client.calls.started")
                               .ToList();
    Assert.Multiple(() =>
                    {
                      Assert.That(perEndpoint,
                                  Has.Count.EqualTo(2),
                                  "a series for each endpoint");
                      Assert.That(collector.Value("armonik.client.calls.started",
                                                  ofOne),
                                  Is.EqualTo(2));
                      Assert.That(collector.Value("armonik.client.calls.started",
                                                  ofTwo),
                                  Is.EqualTo(3));
                      Assert.That(collector.Value("armonik.client.messages.sent",
                                                  ofTwo),
                                  Is.EqualTo(3));
                      Assert.That(collector.Value("armonik.client.dials",
                                                  ofOne.Concat(new[]
                                                                    {
                                                                      ("armonik.dial.outcome", "succeeded"),
                                                                    })
                                                       .ToArray()),
                                  Is.EqualTo(1));
                      Assert.That(collector.Measurements.Where(measurement => measurement.Instrument == "armonik.client.host.memory.waits"),
                                  Is.Not.Empty.And.All.Matches<(string Instrument, double Value, Dictionary<string, object?> Tags)>(measurement => !measurement.Tags.ContainsKey("server.address")),
                                  "the memory ceiling is the runtime's, not an endpoint's");
                    });

    await first.DisposeAsync()
               .ConfigureAwait(false);
    collector.Collect();
    Assert.Multiple(() =>
                    {
                      Assert.That(collector.Value("armonik.client.calls.started",
                                                  ofOne),
                                  Is.EqualTo(0),
                                  "an endpoint with no open channel has no series");
                      Assert.That(collector.Value("armonik.client.calls.started",
                                                  ofTwo),
                                  Is.EqualTo(3));
                    });
  }

  [Test]
  public async Task AFailedCallIsCountedByItsStatusAndDerivedAsFailed()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    using var collector = new Collector();
    await using var channel = Runtime.Channel(Endpoint);

    Assert.Throws<RpcException>(() => Client(channel)
                                  .Refuse(new EchoRequest
                                          {
                                            Text = "x",
                                          }));
    await Say(channel,
              1)
      .ConfigureAwait(false);
    collector.Collect();

    Assert.Multiple(() =>
                    {
                      Assert.That(collector.Value("armonik.client.calls.ended",
                                                  ("rpc.response.status_code", "PERMISSION_DENIED")),
                                  Is.EqualTo(1));
                      Assert.That(collector.Value("armonik.client.calls.failed"),
                                  Is.EqualTo(1));
                      Assert.That(collector.Value("armonik.client.calls.deadline_exceeded"),
                                  Is.EqualTo(0));
                      Assert.That(collector.Value("armonik.client.calls.unimplemented"),
                                  Is.EqualTo(0));
                    });
  }

  [Test]
  public async Task AStreamThatIsOpenIsCurrentAtEachCollection()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    using var collector = new Collector();
    await using var channel = Runtime.Channel(Endpoint);

    using var call = Client(channel)
      .Chat();
    await call.RequestStream.WriteAsync(new EchoRequest
                                        {
                                          Text = "one",
                                        })
              .ConfigureAwait(false);
    Assert.That(await call.ResponseStream.MoveNext()
                          .ConfigureAwait(false));
    collector.Collect();
    var first = (collector.Value("armonik.client.messages.sent"), collector.Value("armonik.client.calls.active"));

    await call.RequestStream.WriteAsync(new EchoRequest
                                        {
                                          Text = "two",
                                        })
              .ConfigureAwait(false);
    Assert.That(await call.ResponseStream.MoveNext()
                          .ConfigureAwait(false));
    collector.Collect();
    var second = (collector.Value("armonik.client.messages.sent"), collector.Value("armonik.client.calls.active"));

    await call.RequestStream.CompleteAsync()
              .ConfigureAwait(false);
    Assert.Multiple(() =>
                    {
                      Assert.That(first,
                                  Is.EqualTo((1.0, 1.0)),
                                  "one message, and the call still open");
                      Assert.That(second,
                                  Is.EqualTo((2.0, 1.0)));
                    });
  }

  [Test]
  public async Task AStreamTheServerResetsIsCountedByItsErrorCode()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    using var collector = new Collector();
    await using var channel = Runtime.Channel(Endpoint);

    Assert.Throws<RpcException>(() => Client(channel)
                                  .Reset(new EchoRequest
                                         {
                                           Text = "before the head",
                                         }));
    collector.Collect();

    Assert.That(collector.Value("armonik.client.streams.reset",
                                ("http2.reset.reason", "ENHANCE_YOUR_CALM")),
                Is.EqualTo(1));
  }

  [Test]
  public async Task ACollectionThatComesLaterSeesTheCallsOfBefore()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    // Nothing listens while the calls are made: the instruments exist and are read when asked.
    await using var channel = Runtime.Channel(Endpoint);
    await Say(channel,
              2)
      .ConfigureAwait(false);

    using var collector = new Collector();
    collector.Collect();

    Assert.That(collector.Value("armonik.client.calls.started"),
                Is.EqualTo(2));
  }

  [Test]
  public async Task TheInstrumentsAreGoneWhenTheRuntimeIs()
  {
    if (!Counts)
    {
      Assert.Ignore("this run is against the build with no counters");
    }

    await RestartAsync(() => NativeRuntime.Create())
      .ConfigureAwait(false);
    await RestartAsync(() => NativeRuntime.Create())
      .ConfigureAwait(false);

    using var collector = new Collector();
    collector.Collect();

    Assert.Multiple(() =>
                    {
                      Assert.That(collector.Instruments.Count(name => name == "armonik.client.calls.started"),
                                  Is.EqualTo(1),
                                  "one runtime lives, so one set of instruments does");
                      Assert.That(collector.Value("armonik.client.calls.started"),
                                  Is.EqualTo(0),
                                  "and it counts from nothing");
                    });
  }

  /// <summary>The dropped logs are the binding's, so they are registered whichever build is loaded.</summary>
  [Test]
  public async Task TheLogsTheBindingDroppedAreCountedWhateverTheBuild()
  {
    await RestartAsync(() => NativeRuntime.Create(0,
                                                  0,
                                                  NullLoggerFactory.Instance))
      .ConfigureAwait(false);

    using var collector = new Collector();
    collector.Collect();

    Assert.Multiple(() =>
                    {
                      Assert.That(collector.Instruments,
                                  Does.Contain("armonik.client.logs.dropped"));
                      Assert.That(collector.Value("armonik.client.logs.dropped"),
                                  Is.EqualTo(0));
                      Assert.That(collector.Instruments.Contains("armonik.client.calls.started"),
                                  Is.EqualTo(Counts),
                                  "the engine's own are there only when it counts");
                    });
  }

  [Test]
  public async Task TheNamesFollowTheGuidanceOfOpenTelemetry()
  {
    // A log makes the binding's own instrument exist whichever build is loaded.
    await RestartAsync(() => NativeRuntime.Create(0,
                                                  0,
                                                  NullLoggerFactory.Instance))
      .ConfigureAwait(false);
    using var collector = new Collector();

    Assert.That(collector.Instruments,
                Is.Not.Empty);

    Assert.That(collector.Instruments,
                Has.All.Matches<string>(name => System.Text.RegularExpressions.Regex.IsMatch(name,
                                                                                             "^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)+$")),
                "lowercase, dot-separated, no dashes and no unit");
  }

  [Test]
  public void AnEndpointIsTaggedWithItsHostAndItsPortWhenItStatesOne()
  {
    static string Show(string endpoint)
      => string.Join(",",
                     EngineMetrics.ServerTags(endpoint)
                                  .Select(tag => $"{tag.Key}={tag.Value}"));

    Assert.Multiple(() =>
                    {
                      Assert.That(Show("127.0.0.1:5001"),
                                  Is.EqualTo("server.address=127.0.0.1,server.port=5001"));
                      Assert.That(Show("armonik.example"),
                                  Is.EqualTo("server.address=armonik.example"),
                                  "no port stated, none tagged");
                      Assert.That(Show("[::1]:5001"),
                                  Is.EqualTo("server.address=::1,server.port=5001"));
                      Assert.That(Show("[::1]"),
                                  Is.EqualTo("server.address=::1"));
                      Assert.That(Show("host:notaport"),
                                  Is.EqualTo("server.address=host"));
                    });
  }

  [Test]
  public void ARetrySlotNamesWhatItCounts()
  {
    Assert.Multiple(() =>
                    {
                      Assert.That(EngineMetrics.RetrySlot(13),
                                  Is.EqualTo(("status", "UNAVAILABLE")));
                      Assert.That(EngineMetrics.RetrySlot(16),
                                  Is.EqualTo(("http", "408")));
                      Assert.That(EngineMetrics.RetrySlot(22),
                                  Is.EqualTo(("http", "other")));
                      Assert.That(EngineMetrics.RetrySlot(23 + 11),
                                  Is.EqualTo(("reset", "ENHANCE_YOUR_CALM")));
                      Assert.That(EngineMetrics.RetrySlot(37),
                                  Is.EqualTo(("reset", "other")));
                      Assert.That(EngineMetrics.RetrySlot(38),
                                  Is.EqualTo(("pushback", (string?)null)));
                      Assert.That(EngineMetrics.RetrySlot(39),
                                  Is.EqualTo(("dial", (string?)null)));
                      Assert.That(EngineMetrics.RetrySlot(40),
                                  Is.EqualTo(("connection", (string?)null)));
                    });
  }
}
