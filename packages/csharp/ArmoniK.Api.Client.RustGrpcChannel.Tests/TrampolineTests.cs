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
using System.Runtime.InteropServices;

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class TrampolineTests
{
  /// <summary>A terminal whose publish throws still lets its call go.</summary>
  [Test]
  public unsafe void ATerminalWhosePublishThrowsStillLetsItsCallGo()
  {
    var sink  = new ThrowingSink();
    var root  = GCHandle.Alloc(sink);
    var raised = Marshal.AllocHGlobal(Marshal.SizeOf<ak_event>());
    try
    {
      // A status with no payload, which `ak_event_consumed` answers as a no-op.
      Marshal.StructureToPtr(new ak_event
                             {
                               kind = ak_event_kind.AK_EVENT_STATUS,
                             },
                             raised,
                             false);

      NativeRuntime.OnEvent(null,
                            (void*)GCHandle.ToIntPtr(root),
                            (ak_event*)raised,
                            1);

      Assert.That(sink.Returned,
                  Is.True);
    }
    finally
    {
      Marshal.FreeHGlobal(raised);
      root.Free();
    }
  }

  /// <summary>The events of one callback are published in order, and the reader woken once.</summary>
  [Test]
  public unsafe void ACallbacksEventsArePublishedInOrderAndSignalledOnce()
  {
    var sink   = new CountingSink();
    var root   = GCHandle.Alloc(sink);
    var size   = Marshal.SizeOf<ak_event>();
    var raised = Marshal.AllocHGlobal(3 * size);
    try
    {
      var kinds = new[]
                  {
                    ak_event_kind.AK_EVENT_INITIAL_METADATA,
                    ak_event_kind.AK_EVENT_MESSAGE,
                    ak_event_kind.AK_EVENT_STATUS,
                  };
      for (var at = 0; at < kinds.Length; at++)
      {
        Marshal.StructureToPtr(new ak_event
                               {
                                 kind = kinds[at],
                               },
                               raised + at * size,
                               false);
      }

      NativeRuntime.OnEvent(null,
                            (void*)GCHandle.ToIntPtr(root),
                            (ak_event*)raised,
                            3);

      Assert.That(sink.Published,
                  Is.EqualTo(kinds));
      Assert.That(sink.Arrivals,
                  Is.EqualTo(1));
      Assert.That(sink.Returned,
                  Is.True);
    }
    finally
    {
      Marshal.FreeHGlobal(raised);
      root.Free();
    }
  }

  /// <summary>An acquittal wakes no reader: the ring never sees it.</summary>
  [Test]
  public unsafe void AnAcquittalWakesNoReader()
  {
    var sink   = new CountingSink();
    var root   = GCHandle.Alloc(sink);
    var raised = Marshal.AllocHGlobal(Marshal.SizeOf<ak_event>());
    try
    {
      Marshal.StructureToPtr(new ak_event
                             {
                               kind = ak_event_kind.AK_EVENT_WRITE_DONE,
                             },
                             raised,
                             false);

      NativeRuntime.OnEvent(null,
                            (void*)GCHandle.ToIntPtr(root),
                            (ak_event*)raised,
                            1);

      Assert.That(sink.Arrivals,
                  Is.EqualTo(0));
    }
    finally
    {
      Marshal.FreeHGlobal(raised);
      root.Free();
    }
  }

  private sealed class ThrowingSink : ICallSink
  {
    public bool Returned { get; private set; }

    public void TerminalReturned()
      => Returned = true;

    public void Cancel()
    {
    }

    public bool Publish(ak_event_kind kind,
                        in ak_bytes   payload,
                        int           statusCode)
      => throw new InvalidOperationException("a publish that fails");

    public void Arrived()
    {
    }
  }

  private sealed class CountingSink : ICallSink
  {
    public List<ak_event_kind> Published { get; } = new();

    public int Arrivals { get; private set; }

    public bool Returned { get; private set; }

    public void TerminalReturned()
      => Returned = true;

    public void Cancel()
    {
    }

    // Taken as the ring takes a data event, and nothing else.
    public bool Publish(ak_event_kind kind,
                        in ak_bytes   payload,
                        int           statusCode)
    {
      Published.Add(kind);
      return kind is not (ak_event_kind.AK_EVENT_WRITE_DONE or ak_event_kind.AK_EVENT_BUDGET_WAKE);
    }

    public void Arrived()
      => Arrivals++;
  }
}
