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

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>The response stream of a server streaming or duplex call.</summary>
/// <remarks>It holds nothing: the reader machine is the call's, because the ring, the drain and
/// the terminal are, and a second place to keep a phase in would be a second thing to keep
/// consistent.</remarks>
internal sealed class NativeResponseStream<TResponse> : IAsyncStreamReader<TResponse>
  where TResponse : class
{
  private readonly NativeCall<TResponse> call_;

  internal NativeResponseStream(NativeCall<TResponse> call)
    => call_ = call;

  public TResponse Current
    => call_.Current;

  public Task<bool> MoveNext(CancellationToken cancellationToken)
    => call_.MoveNext(cancellationToken);
}
