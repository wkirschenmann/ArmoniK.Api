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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What `armonik-transport-ffi/tests/layout.rs` asserts, from the other side.</summary>
///
/// The declarations here are rendered from the Rust, so they agree with it by construction. That
/// says nothing about what the CLR does with them: `Marshal.SizeOf` and
/// `Marshal.OffsetOf` measure the layout this runtime gives a declaration, per target framework
/// and per architecture, and a declaration right in C# and laid out otherwise is read as whatever
/// the other side's bytes happened to mean. This fixture runs under whichever architecture the
/// test host was built for, which is where such a mistake shows.
[TestFixture]
public class AbiLayoutTests
{
  private static readonly int Ptr = IntPtr.Size;

  [Test]
  public void ABorrowedViewIsAPointerAndALength()
    => Assert.That(Marshal.SizeOf<ak_bytes_in>(),
                   Is.EqualTo(2 * Ptr));

  [Test]
  public void AnOwnedViewAndALentBufferHaveTheSameShape()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<ak_bytes>(),
                                     Is.EqualTo(3 * Ptr));
                         Assert.That(Offset<ak_bytes>("len"),
                                     Is.EqualTo(Ptr));
                         Assert.That(Offset<ak_bytes>("owner"),
                                     Is.EqualTo(2 * Ptr));

                         Assert.That(Marshal.SizeOf<ak_buffer>(),
                                     Is.EqualTo(3 * Ptr));
                         Assert.That(Offset<ak_buffer>("owner"),
                                     Is.EqualTo(2 * Ptr));
                       });

  [Test]
  public void AnEventCarriesItsPayloadInline()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<ak_event>(),
                                     Is.EqualTo(4 * Ptr + 8));
                         Assert.That(Offset<ak_event>("kind"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<ak_event>("payload"),
                                     Is.EqualTo(Ptr));
                         Assert.That(Offset<ak_event>("status_code"),
                                     Is.EqualTo(4 * Ptr));
                         Assert.That(Offset<ak_event>("host_debt"),
                                     Is.EqualTo(4 * Ptr + 4));
                       });

  [Test]
  public void EveryEnumTheAbiCrossesIsAnInt()
    => Assert.Multiple(() =>
                       {
                         foreach (var crossing in new[]
                                                  {
                                                    typeof(ak_status),
                                                    typeof(ak_runtime_state),
                                                    typeof(ak_event_kind),
                                                    typeof(ak_host_debt),
                                                    typeof(ak_channel_state),
                                                    typeof(ak_head_origin),
                                                  })
                         {
                           Assert.That(Enum.GetUnderlyingType(crossing),
                                       Is.EqualTo(typeof(int)),
                                       crossing.Name);
                         }
                       });

  [Test]
  public void AnOptionsStructStartsWithTheFieldsThatVersionIt()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<ak_runtime_config>(),
                                     Is.EqualTo(32 + 2 * Ptr));
                         Assert.That(Offset<ak_runtime_config>("struct_size"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<ak_runtime_config>("version"),
                                     Is.EqualTo(4));
                         Assert.That(Offset<ak_runtime_config>("flags"),
                                     Is.EqualTo(8));
                         Assert.That(Offset<ak_runtime_config>("reserved"),
                                     Is.EqualTo(12));
                         Assert.That(Offset<ak_runtime_config>("memory_ceiling"),
                                     Is.EqualTo(16));
                         Assert.That(Offset<ak_runtime_config>("memory_hard_ceiling"),
                                     Is.EqualTo(24));
                         Assert.That(Offset<ak_runtime_config>("channel_defaults_json"),
                                     Is.EqualTo(32));

                         Assert.That(Marshal.SizeOf<ak_call_start_options>(),
                                     Is.EqualTo(16 + 4 * Ptr + 8));
                         Assert.That(Offset<ak_call_start_options>("struct_size"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<ak_call_start_options>("version"),
                                     Is.EqualTo(4));
                         Assert.That(Offset<ak_call_start_options>("flags"),
                                     Is.EqualTo(8));
                         Assert.That(Offset<ak_call_start_options>("reserved"),
                                     Is.EqualTo(12));
                         Assert.That(Offset<ak_call_start_options>("method"),
                                     Is.EqualTo(16));
                         Assert.That(Offset<ak_call_start_options>("metadata"),
                                     Is.EqualTo(16 + 2 * Ptr));
                         Assert.That(Offset<ak_call_start_options>("timeout_ns"),
                                     Is.EqualTo(16 + 4 * Ptr));
                       });

  [Test]
  public void TheObservationalStructsArePlainIntegers()
    => Assert.Multiple(() =>
                       {
                         Assert.That(Marshal.SizeOf<ak_call_debt>(),
                                     Is.EqualTo(16));
                         Assert.That(Offset<ak_call_debt>("payloads_owed"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<ak_call_debt>("buffers_lent"),
                                     Is.EqualTo(4));
                         Assert.That(Offset<ak_call_debt>("callbacks_in_flight"),
                                     Is.EqualTo(8));
                         Assert.That(Offset<ak_call_debt>("terminal_delivered"),
                                     Is.EqualTo(12));

                         Assert.That(Marshal.SizeOf<ak_memory_usage>(),
                                     Is.EqualTo(16));
                         Assert.That(Offset<ak_memory_usage>("bytes_used"),
                                     Is.EqualTo(0));
                         Assert.That(Offset<ak_memory_usage>("ceiling"),
                                     Is.EqualTo(8));
                       });

  /// <summary>The discriminants, which no size or offset would catch.</summary>
  [Test]
  public void EveryDiscriminantIsTheOneTheHeaderGives()
    => Assert.Multiple(() =>
                       {
                         Assert.That((int)ak_status.AK_STATUS_OK,
                                     Is.EqualTo(0));
                         Assert.That((int)ak_status.AK_STATUS_HANDLE_STALE,
                                     Is.EqualTo(1));
                         Assert.That((int)ak_status.AK_STATUS_SLOT_BUSY,
                                     Is.EqualTo(2));
                         Assert.That((int)ak_status.AK_STATUS_INVALID_ARG,
                                     Is.EqualTo(3));
                         Assert.That((int)ak_status.AK_STATUS_INTERNAL,
                                     Is.EqualTo(4));
                         Assert.That((int)ak_status.AK_STATUS_BUDGET_BUSY,
                                     Is.EqualTo(5));
                         Assert.That((int)ak_status.AK_STATUS_INVALID_STATE,
                                     Is.EqualTo(6));
                         Assert.That((int)ak_status.AK_STATUS_MESSAGE_TOO_LARGE,
                                     Is.EqualTo(7));
                         Assert.That((int)ak_status.AK_STATUS_CORRUPTED,
                                     Is.EqualTo(8));

                         Assert.That((int)ak_runtime_state.AK_RUNTIME_NONE,
                                     Is.EqualTo(0));
                         Assert.That((int)ak_runtime_state.AK_RUNTIME_RUNNING,
                                     Is.EqualTo(1));
                         Assert.That((int)ak_runtime_state.AK_RUNTIME_GRPC_STOPPING,
                                     Is.EqualTo(2));
                         Assert.That((int)ak_runtime_state.AK_RUNTIME_GRPC_STOPPED,
                                     Is.EqualTo(3));
                         Assert.That((int)ak_runtime_state.AK_RUNTIME_QUIESCENT,
                                     Is.EqualTo(4));
                         Assert.That((int)ak_runtime_state.AK_RUNTIME_FAILED_UNQUIESCED,
                                     Is.EqualTo(5));

                         Assert.That((int)ak_channel_state.AK_CHANNEL_NONE,
                                     Is.EqualTo(0));
                         Assert.That((int)ak_channel_state.AK_CHANNEL_OPEN,
                                     Is.EqualTo(1));
                         Assert.That((int)ak_channel_state.AK_CHANNEL_CLOSING,
                                     Is.EqualTo(2));
                         Assert.That((int)ak_channel_state.AK_CHANNEL_CLOSED,
                                     Is.EqualTo(3));

                         Assert.That((int)ak_event_kind.AK_EVENT_INITIAL_METADATA,
                                     Is.EqualTo(1));
                         Assert.That((int)ak_event_kind.AK_EVENT_MESSAGE,
                                     Is.EqualTo(2));
                         Assert.That((int)ak_event_kind.AK_EVENT_STATUS,
                                     Is.EqualTo(3));
                         Assert.That((int)ak_event_kind.AK_EVENT_WRITE_DONE,
                                     Is.EqualTo(4));
                         Assert.That((int)ak_event_kind.AK_EVENT_SHUTDOWN_COMPLETE,
                                     Is.EqualTo(5));
                         Assert.That((int)ak_event_kind.AK_EVENT_RESOURCES_RELEASED,
                                     Is.EqualTo(6));
                         Assert.That((int)ak_event_kind.AK_EVENT_BUDGET_WAKE,
                                     Is.EqualTo(7));

                         Assert.That((int)ak_host_debt.AK_HOST_NOTHING_TO_RETURN,
                                     Is.EqualTo(0));
                         Assert.That((int)ak_host_debt.AK_HOST_MUST_RETURN,
                                     Is.EqualTo(1));

                         Assert.That((int)ak_head_origin.AK_HEAD_RECEIVED,
                                     Is.EqualTo(0));
                         Assert.That((int)ak_head_origin.AK_HEAD_TRAILERS_ONLY,
                                     Is.EqualTo(1));
                         Assert.That((int)ak_head_origin.AK_HEAD_NO_RESPONSE,
                                     Is.EqualTo(2));
                       });

  /// <summary>The version this binding was written against, as a literal.</summary>
  /// <remarks>Not a check against the header - both are rendered from the same Rust constant, and
  /// this compares that constant to the number it is. What it catches is the constant being
  /// edited without anyone meaning to.</remarks>
  [Test]
  public void TheAbiVersionThisBindingSpeaksIsOne()
    => Assert.That(NativeMethods.AK_ABI_VERSION,
                   Is.EqualTo(1));

  private static int Offset<T>(string field)
    => Marshal.OffsetOf<T>(field)
              .ToInt32();
}
