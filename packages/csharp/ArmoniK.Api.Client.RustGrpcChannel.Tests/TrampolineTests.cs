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
                            (ak_event*)raised);

      Assert.That(sink.Returned,
                  Is.True);
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
  }
}
