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

using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>What one call has been delivered and has not given back.</summary>
///
/// A bounded queue of the delivery window and one slot for the terminal, written by the engine's
/// callback thread and read by one consumer. Two jobs, and they are the same array on purpose:
/// the bound is what tells the engine the host is not keeping up, and the occupancy is the record
/// of what is still owed - a payload stays the engine's until this releases it, so the queue is
/// where an event waits for a consumer *and* what a drain finds if the reader goes away
/// mid-decode.
///
/// The borrow is the reason nothing here dequeues: the slot the consumer is decoding stays in the
/// queue, at the tail, until <see cref="Release" /> gives its payload back. That is the lifetime
/// the model states as `ParsingReadOwnsItsSlot`.
internal sealed class DeliveryRing
{
  internal struct Slot
  {
    internal ak_bytes Payload;
    internal ak_event_kind Kind;

    // A gRPC status on a terminal, an ak_head_origin on a head.
    internal int Status;
  }

  private readonly Slot[] slots_;
  private readonly int mask_;
  private long head_;
  private long tail_;

  private readonly ArrivalSignal arrived_ = new();

  internal DeliveryRing(int deliveryCredits)
  {
    // One slot more than the window, because the terminal goes out with every credit spent.
    //
    // `NativeRuntime.MaxDeliveryCredits` is what keeps this loop finite: a shift is not
    // checked in C#, so an unbounded window would take `size` through `int.MinValue` to zero and
    // spin here for ever on the caller's thread.
    var size = 1;
    while (size < deliveryCredits + 1)
    {
      size <<= 1;
    }

    slots_ = new Slot[size];
    mask_  = size - 1;
  }

  /// <summary>Takes an event from the engine's callback thread, waking nobody.</summary>
  /// <remarks>From the volatile write the slot is the consumer's, and so is giving the payload
  /// back. A callback stores every event it carries and then calls <see cref="Arrived" /> once,
  /// so a consumer woken by the first does not wake again for the next.</remarks>
  internal void Store(ak_event_kind kind,
                      in ak_bytes payload,
                      int statusCode)
  {
    var at = (int)(head_ & mask_);
    slots_[at].Payload = payload;
    slots_[at].Kind    = kind;
    slots_[at].Status  = statusCode;

    Volatile.Write(ref head_,
                   head_ + 1);
  }

  /// <summary>Wakes whoever waits for what was stored.</summary>
  internal void Arrived()
    => arrived_.Set();

  /// <summary>Whether the queue holds nothing the consumer has not seen.</summary>
  internal bool IsEmpty
    => Volatile.Read(ref head_) == tail_;

  /// <summary>The slot the consumer owns next, which stays here until <see cref="Release" />.</summary>
  internal bool TryPeek(out Slot slot)
  {
    if (IsEmpty)
    {
      slot = default;
      return false;
    }

    slot = slots_[(int)(tail_ & mask_)];
    return true;
  }

  /// <summary>The slot after the one <see cref="TryPeek" /> answers, which stays the ring's.</summary>
  internal bool TryPeekBehind(out Slot slot)
  {
    if (Volatile.Read(ref head_) - tail_ < 2)
    {
      slot = default;
      return false;
    }

    slot = slots_[(int)((tail_ + 1) & mask_)];
    return true;
  }

  /// <summary>Gives the peeked payload back to the engine and moves past it.</summary>
  /// <remarks>Where every payload the queue accepted is returned, and the only place: the release
  /// is FIFO by ABI rule, so the tail is what says which one is owed. The trampoline returns the
  /// ones no queue took, and that is the other half of the same rule. The tail is not volatile -
  /// one consumer advances it, and what orders it against a reader observing a new phase is the
  /// phase's own publication.</remarks>
  internal void Release()
  {
    NativeMethods.ak_event_consumed(slots_[(int)(tail_ & mask_)].Payload);
    tail_++;
  }

  /// <summary>A wait for the next arrival, taken before looking at the queue.</summary>
  internal Task NextArrival()
    => arrived_.Next();

  /// <summary>Wakes whoever waits, granting nothing.</summary>
  /// <remarks>What a cancelled call needs: whoever waits - a read, or a prologue with no read
  /// behind it - has to look again at state that is not this queue's.</remarks>
  internal void Wake()
    => arrived_.Set();
}
