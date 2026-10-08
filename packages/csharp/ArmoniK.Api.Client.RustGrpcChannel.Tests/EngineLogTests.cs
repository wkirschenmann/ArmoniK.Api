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
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

using Microsoft.Extensions.Logging;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The engine's logs, as a host's <see cref="ILoggerFactory" /> receives them.</summary>
[TestFixture]
public class EngineLogTests
{
  private sealed record Seen(string                                          Category,
                             LogLevel                                        Level,
                             string                                          Text,
                             IReadOnlyList<KeyValuePair<string, object?>>?   State,
                             Exception?                                      Error,
                             int                                             Thread);

  /// <summary>A factory that keeps what it is given, and can be made to fail or to wait.</summary>
  private sealed class CollectingFactory : ILoggerFactory
  {
    private readonly ConcurrentQueue<Seen> seen_ = new();

    /// <summary>How many of the next records throw instead of being kept.</summary>
    public int Throwing;

    /// <summary>When set, a write waits for it, as a slow provider does.</summary>
    public ManualResetEventSlim? Holding;

    public IReadOnlyCollection<Seen> All
      => seen_;

    public ILogger CreateLogger(string categoryName)
      => new Logger(this,
                    categoryName);

    public void AddProvider(ILoggerProvider provider)
    {
    }

    public void Dispose()
    {
    }

    public async Task<Seen> WaitFor(Func<Seen, bool> wanted)
    {
      var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(10);
      while (DateTime.UtcNow < deadline)
      {
        var found = seen_.FirstOrDefault(wanted);
        if (found is not null)
        {
          return found;
        }

        await Task.Delay(5)
                  .ConfigureAwait(false);
      }

      throw new AssertionException($"no such record in 10 s; there are {seen_.Count}: {string.Join(" | ", seen_.Select(seen => seen.Text))}");
    }

    private sealed class Logger : ILogger
    {
      private readonly CollectingFactory factory_;

      private readonly string category_;

      public Logger(CollectingFactory factory,
                    string            category)
      {
        factory_  = factory;
        category_ = category;
      }

      public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull
        => null;

      public bool IsEnabled(LogLevel logLevel)
        => true;

      public void Log<TState>(LogLevel                         logLevel,
                              EventId                          eventId,
                              TState                           state,
                              Exception?                       exception,
                              Func<TState, Exception?, string> formatter)
      {
        factory_.Holding?.Wait();
        if (Interlocked.Decrement(ref factory_.Throwing) >= 0)
        {
          throw new InvalidOperationException("a provider that throws");
        }

        factory_.seen_.Enqueue(new Seen(category_,
                                        logLevel,
                                        formatter(state,
                                                  exception),
                                        state as IReadOnlyList<KeyValuePair<string, object?>>,
                                        exception,
                                        Environment.CurrentManagedThreadId));
      }
    }
  }

  private static async Task Run(Func<ILoggerFactory, NativeRuntime> start,
                                CollectingFactory                   factory,
                                Func<NativeRuntime, Task>           body)
  {
    var runtime = start(factory);
    try
    {
      await body(runtime)
        .ConfigureAwait(false);
    }
    finally
    {
      await runtime.DisposeAsync()
                   .ConfigureAwait(false);
    }
  }

  [Test]
  public async Task AnUnknownKeyIsLoggedAtInformationByTheEngineWithItsSourceAndPath()
  {
    var factory = new CollectingFactory();
    await Run(logs => NativeRuntime.Create(new NativeConfiguration("").LoadConfigFromCommandLine(new[]
                                                                                                 {
                                                                                                   "--Misspelled=1",
                                                                                                 }),
                                           logs),
              factory,
              async _ =>
              {
                var seen = await factory.WaitFor(entry => entry.Text.Contains("does not know"))
                                        .ConfigureAwait(false);
                Assert.Multiple(() =>
                                {
                                  Assert.That(seen.Level,
                                              Is.EqualTo(LogLevel.Information));
                                  Assert.That(seen.Category,
                                              Is.EqualTo("armonik_transport::configuration"));
                                  Assert.That(seen.State!.Any(pair => pair.Key == "key" && (string?)pair.Value == "Misspelled"),
                                              Is.True,
                                              seen.Text);
                                  Assert.That(seen.State!.Any(pair => pair.Key == "source" && (string?)pair.Value == "pairs"),
                                              Is.True,
                                              seen.Text);
                                  Assert.That(seen.Text,
                                              Does.Contain("key=Misspelled"));
                                  Assert.That(seen.Thread,
                                              Is.Not.EqualTo(Environment.CurrentManagedThreadId),
                                              "written from a thread of the binding's, never the engine's or the caller's");
                                });
              })
      .ConfigureAwait(false);
  }

  [Test]
  public async Task TheFilterOptionSelectsWhatReachesTheLogger()
  {
    var factory = new CollectingFactory();
    await Run(logs => NativeRuntime.Create(new RuntimeOptions
                                           {
                                             Logging = new LoggingOptions
                                                       {
                                                         Filter = "armonik_transport=debug",
                                                       },
                                           },
                                           logs),
              factory,
              async runtime =>
              {
                await using (runtime.Channel("http://127.0.0.1:1"))
                {
                  var seen = await factory.WaitFor(entry => entry.Text.Contains("channel created"))
                                          .ConfigureAwait(false);
                  Assert.That(seen.Level,
                              Is.EqualTo(LogLevel.Debug));
                }
              })
      .ConfigureAwait(false);
  }

  [Test]
  public async Task TheDefaultFilterKeepsDebugOut()
  {
    var factory = new CollectingFactory();
    await Run(logs => NativeRuntime.Create(0,
                                           0,
                                           logs),
              factory,
              async runtime =>
              {
                await using (runtime.Channel("http://127.0.0.1:1"))
                {
                  await factory.WaitFor(entry => entry.Text.Contains("runtime's effective configuration"))
                               .ConfigureAwait(false);
                }
              })
      .ConfigureAwait(false);

    Assert.That(factory.All.Select(seen => seen.Level),
                Is.All.GreaterThanOrEqualTo(LogLevel.Information));
  }

  [Test]
  public async Task AProviderThatThrowsCostsItsRecordAndNotTheWriter()
  {
    var factory = new CollectingFactory
                  {
                    Throwing = 1,
                  };
    await Run(logs => NativeRuntime.Create(new RuntimeOptions
                                           {
                                             Logging = new LoggingOptions
                                                       {
                                                         Filter = "armonik_transport*=debug",
                                                       },
                                           },
                                           logs),
              factory,
              async runtime =>
              {
                await using (runtime.Channel("http://127.0.0.1:1"))
                {
                  await factory.WaitFor(entry => entry.Text.Contains("channel created"))
                               .ConfigureAwait(false);
                }
              })
      .ConfigureAwait(false);
  }

  [Test]
  public async Task DisposingTheRuntimeWritesWhatIsQueuedAndTheLogGoesQuiet()
  {
    var factory = new CollectingFactory();
    var runtime = NativeRuntime.Create(0,
                                       0,
                                       factory);
    Assert.That(EngineLog.Current,
                Is.Not.Null);
    await runtime.DisposeAsync()
                 .ConfigureAwait(false);

    Assert.That(EngineLog.Current,
                Is.Null);
    Assert.That(factory.All.Select(seen => seen.Text),
                Has.Some.Contains("runtime's effective configuration"),
                "the record the creation logged is written before the disposal returns");
  }

  /// <summary>A creation refused because a runtime lives leaves that runtime's log in place.</summary>
  [Test]
  public async Task ASecondCreationTheEngineRefusesLeavesTheLiveRuntimesLogCurrent()
  {
    var first  = new CollectingFactory();
    var second = new CollectingFactory();
    var runtime = NativeRuntime.Create(0,
                                       0,
                                       first);
    try
    {
      var current = EngineLog.Current;
      Assert.That(current,
                  Is.Not.Null);

      Assert.That(() => NativeRuntime.Create(0,
                                             0,
                                             second),
                  Throws.InvalidOperationException);

      Assert.That(EngineLog.Current,
                  Is.SameAs(current));
    }
    finally
    {
      await runtime.DisposeAsync()
                   .ConfigureAwait(false);
    }
  }

  [Test]
  public void ACreationTheEngineRefusesStillWritesWhatItsLoadLogged()
  {
    var factory = new CollectingFactory();
    var configuration = new NativeConfiguration("").LoadConfigFromCommandLine(new[]
                                                                               {
                                                                                 "--Aaa=1",
                                                                                 "--MemoryCeiling=many",
                                                                               });

    Assert.That(() => NativeRuntime.Create(configuration,
                                           factory),
                Throws.InvalidOperationException);

    Assert.That(factory.All.Select(seen => seen.Text),
                Has.Some.Contains("does not know"));
    Assert.That(EngineLog.Current,
                Is.Null);
  }

  [Test]
  public async Task ARecordIsMappedToItsLevelItsCategoryItsTextAndItsFields()
  {
    var factory = new CollectingFactory();
    var log     = new EngineLog(factory);
    try
    {
      LogEmitter.Emit(log,
           NativeMethods.AK_LOG_WARN,
           "h2::proto",
           "a message é",
           ("endpoint", "http://host:1"),
           ("empty", string.Empty));
      var seen = await factory.WaitFor(entry => entry.Category == "h2::proto")
                              .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(seen.Level,
                                    Is.EqualTo(LogLevel.Warning));
                        Assert.That(seen.Text,
                                    Is.EqualTo("a message é endpoint=http://host:1 empty="));
                        Assert.That(seen.State!.Select(pair => (pair.Key, (string?)pair.Value)),
                                    Is.EqualTo(new[]
                                               {
                                                 ("endpoint", (string?)"http://host:1"),
                                                 ("empty", string.Empty),
                                                 ("{OriginalFormat}", "a message é"),
                                               }));
                      });
    }
    finally
    {
      log.Close();
    }
  }

  /// <summary>The engine's text is a message, not a template: a structured provider reads
  /// <c>{OriginalFormat}</c> as one, so its braces are doubled.</summary>
  [Test]
  public async Task ABraceInAMessageIsEscapedInTheTemplateAndNotInTheText()
  {
    var factory = new CollectingFactory();
    var log     = new EngineLog(factory);
    try
    {
      LogEmitter.Emit(log,
                      NativeMethods.AK_LOG_INFO,
                      "t",
                      "a {braced} text");
      var seen = await factory.WaitFor(entry => entry.Category == "t")
                              .ConfigureAwait(false);

      Assert.Multiple(() =>
                      {
                        Assert.That(seen.Text,
                                    Is.EqualTo("a {braced} text"));
                        Assert.That(seen.State!.Last()
                                        .Value,
                                    Is.EqualTo("a {{braced}} text"));
                      });
    }
    finally
    {
      log.Close();
    }
  }

  [Test]
  public void EveryLevelOfTheEngineHasOneHere()
  {
    var factory = new CollectingFactory();
    var log     = new EngineLog(factory);
    try
    {
      foreach (var level in new[]
                            {
                              NativeMethods.AK_LOG_ERROR,
                              NativeMethods.AK_LOG_WARN,
                              NativeMethods.AK_LOG_INFO,
                              NativeMethods.AK_LOG_DEBUG,
                              NativeMethods.AK_LOG_TRACE,
                            })
      {
        LogEmitter.Emit(log,
             level,
             "t",
             level.ToString());
      }
    }
    finally
    {
      log.Close();
    }

    Assert.That(factory.All.Select(seen => seen.Level),
                Is.EqualTo(new[]
                           {
                             LogLevel.Error,
                             LogLevel.Warning,
                             LogLevel.Information,
                             LogLevel.Debug,
                             LogLevel.Trace,
                           }));
  }

  [Test]
  public async Task ALoggerSlowerThanTheEngineDropsRecordsAndSaysHowMany()
  {
    var factory = new CollectingFactory
                  {
                    Holding = new ManualResetEventSlim(false),
                  };
    var log = new EngineLog(factory);
    try
    {
      const int Extra = 500;
      for (var index = 0; index < EngineLog.QueueCapacity + Extra; index++)
      {
        LogEmitter.Emit(log,
             NativeMethods.AK_LOG_INFO,
             "t",
             "record");
      }

      factory.Holding.Set();
      var warning = await factory.WaitFor(seen => seen.Text.Contains("were dropped"))
                                 .ConfigureAwait(false);
      var lost = int.Parse(warning.Text.Split(' ')[0]);

      // The writer may hold one record outside the queue when the first is dropped.
      Assert.That(lost,
                  Is.InRange(Extra - 1,
                             Extra + 1));
    }
    finally
    {
      factory.Holding.Set();
      log.Close();
    }
  }

  [Test]
  public async Task AFailureTheBindingCaughtIsLoggedWithItsException()
  {
    var factory = new CollectingFactory();
    var log     = new EngineLog(factory);
    log.Publish();
    try
    {
      EngineLog.Current!.Caught("a call's Publish threw",
                                new InvalidOperationException("boom"));
      var seen = await factory.WaitFor(entry => entry.Text.Contains("Publish threw"))
                              .ConfigureAwait(false);
      Assert.Multiple(() =>
                      {
                        Assert.That(seen.Level,
                                    Is.EqualTo(LogLevel.Error));
                        Assert.That(seen.Error,
                                    Is.InstanceOf<InvalidOperationException>());
                      });
    }
    finally
    {
      log.Close();
    }
  }
}

/// <summary>Calls the binding's callback as the engine does, with a record built here.</summary>
internal static unsafe class LogEmitter
{
  /// <summary>What the engine's callback is given, as the binding receives it, built here.</summary>
  internal static void Emit(EngineLog                    log,
                           uint                         level,
                           string                       target,
                           string                       message,
                           params (string, string)[]    fields)
  {
    var handles = new List<GCHandle>();
    try
    {
      ak_bytes_in View(string text)
      {
        var bytes = Encoding.UTF8.GetBytes(text);
        var pinned = GCHandle.Alloc(bytes,
                                    GCHandleType.Pinned);
        handles.Add(pinned);
        return new ak_bytes_in
               {
                 ptr = bytes.Length == 0
                         ? null
                         : (byte*)pinned.AddrOfPinnedObject(),
                 len = (nuint)bytes.Length,
               };
      }

      var views = fields.Select(field => new ak_log_field
                                         {
                                           key   = View(field.Item1),
                                           value = View(field.Item2),
                                         })
                        .ToArray();
      fixed (ak_log_field* first = views)
      {
        var record = new ak_log_record
                     {
                       struct_size = (uint)sizeof(ak_log_record),
                       level       = level,
                       target      = View(target),
                       message     = View(message),
                       field_count = (nuint)views.Length,
                       fields      = views.Length == 0
                                       ? null
                                       : first,
                     };
        var trampoline = Marshal.GetDelegateForFunctionPointer<NativeMethods.LogCallback>((IntPtr)EngineLog.Trampoline);
        trampoline(log.Context,
                   &record);
      }
    }
    finally
    {
      foreach (var handle in handles)
      {
        handle.Free();
      }
    }
  }
}
