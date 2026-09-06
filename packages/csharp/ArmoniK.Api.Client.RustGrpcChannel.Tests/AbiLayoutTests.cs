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

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What `armonik-transport-ffi/tests/layout.rs` asserts, from the other side.</summary>
///
/// The two declarations of the ABI are written out separately, in Rust and here, and the header
/// is the contract both answer to. Rust pins its own sizes and offsets; nothing pinned these,
/// so a field reordered or widened on one side alone would have been read as whatever the other
/// side's bytes happened to mean. This fixture runs under whichever architecture the test host
/// was built for, which is where such a mistake shows.
[TestFixture]
public class AbiLayoutTests
{
  private static readonly int Ptr = IntPtr.Size;

  [Test]
  public void ABorrowedViewIsAPointerAndALength()
    => Assert.That(Marshal.SizeOf<NativeMethods.AkBytesIn>(),
                   Is.EqualTo(2 * Ptr));

  [Test]
  public void AnOwnedViewAndALentBufferHaveTheSameShape()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<NativeMethods.AkBytes>(),
                                     Is.EqualTo(3 * Ptr));
                         Assert.That(Offset<NativeMethods.AkBytes>("Len"),
                                     Is.EqualTo(Ptr));
                         Assert.That(Offset<NativeMethods.AkBytes>("Owner"),
                                     Is.EqualTo(2 * Ptr));

                         Assert.That(Marshal.SizeOf<NativeMethods.AkBuffer>(),
                                     Is.EqualTo(3 * Ptr));
                         Assert.That(Offset<NativeMethods.AkBuffer>("Owner"),
                                     Is.EqualTo(2 * Ptr));
                       });

  [Test]
  public void AnEventCarriesItsPayloadInline()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<NativeMethods.AkEvent>(),
                                     Is.EqualTo(4 * Ptr + 8));
                         Assert.That(Offset<NativeMethods.AkEvent>("Kind"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<NativeMethods.AkEvent>("Payload"),
                                     Is.EqualTo(Ptr));
                         Assert.That(Offset<NativeMethods.AkEvent>("StatusCode"),
                                     Is.EqualTo(4 * Ptr));
                         Assert.That(Offset<NativeMethods.AkEvent>("HostDebt"),
                                     Is.EqualTo(4 * Ptr + 4));
                       });

  [Test]
  public void EveryEnumTheAbiCrossesIsAnInt()
    => Assert.Multiple(() =>
                       {
                         foreach (var crossing in new[]
                                                  {
                                                    typeof(NativeMethods.AkStatus),
                                                    typeof(NativeMethods.AkRuntimeState),
                                                    typeof(NativeMethods.AkEventKind),
                                                    typeof(NativeMethods.AkHostDebt),
                                                    typeof(NativeMethods.AkChannelState),
                                                  })
                         {
                           Assert.That(Enum.GetUnderlyingType(crossing),
                                       Is.EqualTo(typeof(int)),
                                       crossing.Name);
                         }
                       });

  [Test]
  public void AnOptionsStructStartsWithTheSizeThatVersionsIt()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<NativeMethods.AkRuntimeConfig>(),
                                     Is.EqualTo(16));
                         Assert.That(Offset<NativeMethods.AkRuntimeConfig>("StructSize"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<NativeMethods.AkRuntimeConfig>("WorkerThreads"),
                                     Is.EqualTo(4));
                         Assert.That(Offset<NativeMethods.AkRuntimeConfig>("MemoryCeiling"),
                                     Is.EqualTo(8));

                         Assert.That(Marshal.SizeOf<NativeMethods.AkCallStartOptions>(),
                                     Is.EqualTo(5 * Ptr));
                         Assert.That(Offset<NativeMethods.AkCallStartOptions>("StructSize"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<NativeMethods.AkCallStartOptions>("Method"),
                                     Is.EqualTo(Ptr));
                         Assert.That(Offset<NativeMethods.AkCallStartOptions>("Metadata"),
                                     Is.EqualTo(3 * Ptr));
                       });

  [Test]
  public void TheObservationalStructsArePlainIntegers()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<NativeMethods.AkCallDebt>(),
                                     Is.EqualTo(16));
                         Assert.That(Offset<NativeMethods.AkCallDebt>("PayloadsOwed"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<NativeMethods.AkCallDebt>("BuffersLent"),
                                     Is.EqualTo(4));
                         Assert.That(Offset<NativeMethods.AkCallDebt>("CallbacksInFlight"),
                                     Is.EqualTo(8));
                         Assert.That(Offset<NativeMethods.AkCallDebt>("TerminalDelivered"),
                                     Is.EqualTo(12));

                         Assert.That(Marshal.SizeOf<NativeMethods.AkMemoryUsage>(),
                                     Is.EqualTo(16));
                         Assert.That(Offset<NativeMethods.AkMemoryUsage>("BytesUsed"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<NativeMethods.AkMemoryUsage>("Ceiling"),
                                     Is.EqualTo(8));
                       });

  /// <summary>The discriminants, which no size or offset would catch.</summary>
  [Test]
  public void EveryDiscriminantIsTheOneTheHeaderGives()
    => Assert.Multiple(() =>
                       {
                         Assert.That((int)NativeMethods.AkStatus.Ok,
                                     Is.EqualTo(0));
                         Assert.That((int)NativeMethods.AkStatus.HandleStale,
                                     Is.EqualTo(1));
                         Assert.That((int)NativeMethods.AkStatus.SlotBusy,
                                     Is.EqualTo(2));
                         Assert.That((int)NativeMethods.AkStatus.InvalidArg,
                                     Is.EqualTo(3));
                         Assert.That((int)NativeMethods.AkStatus.Internal,
                                     Is.EqualTo(4));
                         Assert.That((int)NativeMethods.AkStatus.BudgetBusy,
                                     Is.EqualTo(5));
                         Assert.That((int)NativeMethods.AkStatus.InvalidState,
                                     Is.EqualTo(6));
                         Assert.That((int)NativeMethods.AkStatus.MessageTooLarge,
                                     Is.EqualTo(7));

                         Assert.That((int)NativeMethods.AkRuntimeState.None,
                                     Is.EqualTo(0));
                         Assert.That((int)NativeMethods.AkRuntimeState.Running,
                                     Is.EqualTo(1));
                         Assert.That((int)NativeMethods.AkRuntimeState.GrpcStopping,
                                     Is.EqualTo(2));
                         Assert.That((int)NativeMethods.AkRuntimeState.GrpcStopped,
                                     Is.EqualTo(3));
                         Assert.That((int)NativeMethods.AkRuntimeState.Quiescent,
                                     Is.EqualTo(4));
                         Assert.That((int)NativeMethods.AkRuntimeState.FailedUnquiesced,
                                     Is.EqualTo(5));

                         Assert.That((int)NativeMethods.AkChannelState.None,
                                     Is.EqualTo(0));
                         Assert.That((int)NativeMethods.AkChannelState.Open,
                                     Is.EqualTo(1));
                         Assert.That((int)NativeMethods.AkChannelState.Closing,
                                     Is.EqualTo(2));
                         Assert.That((int)NativeMethods.AkChannelState.Closed,
                                     Is.EqualTo(3));

                         Assert.That((int)NativeMethods.AkEventKind.InitialMetadata,
                                     Is.EqualTo(1));
                         Assert.That((int)NativeMethods.AkEventKind.Message,
                                     Is.EqualTo(2));
                         Assert.That((int)NativeMethods.AkEventKind.Status,
                                     Is.EqualTo(3));
                         Assert.That((int)NativeMethods.AkEventKind.WriteDone,
                                     Is.EqualTo(4));
                         Assert.That((int)NativeMethods.AkEventKind.ShutdownComplete,
                                     Is.EqualTo(5));
                         Assert.That((int)NativeMethods.AkEventKind.ResourcesReleased,
                                     Is.EqualTo(6));

                         Assert.That((int)NativeMethods.AkHostDebt.NothingToReturn,
                                     Is.EqualTo(0));
                         Assert.That((int)NativeMethods.AkHostDebt.MustReturn,
                                     Is.EqualTo(1));
                       });

  /// <summary>The version this binding was written against, as a literal.</summary>
  /// <remarks>Not a check against the header - `layout.rs` does that, and this one cannot: it
  /// compares the constant to the number it is. What it catches is the constant being edited
  /// without anyone meaning to.</remarks>
  [Test]
  public void TheAbiVersionThisBindingSpeaksIsOne()
    => Assert.That(NativeMethods.AbiVersion,
                   Is.EqualTo(1));

  private static int Offset<T>(string field)
    => Marshal.OffsetOf<T>(field)
              .ToInt32();
}
