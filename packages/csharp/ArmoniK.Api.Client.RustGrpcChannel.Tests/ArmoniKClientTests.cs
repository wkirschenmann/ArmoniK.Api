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
using System.Threading.Tasks;

using ArmoniK.Api.gRPC.V1;
using ArmoniK.Api.gRPC.V1.Results;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class ArmoniKClientTests
{
  private static string Endpoint
  {
    get
    {
      var endpoint = Environment.GetEnvironmentVariable("GrpcClient__Endpoint");
      if (string.IsNullOrEmpty(endpoint))
      {
        Assert.Ignore("GrpcClient__Endpoint is unset, so no ArmoniK server is running");
      }

      if (!endpoint!.StartsWith("http://",
                                StringComparison.OrdinalIgnoreCase))
      {
        Assert.Ignore($"`{endpoint}` is not plain HTTP/2, which is all this engine speaks");
      }

      return endpoint!;
    }
  }

  [TearDown]
  public void EveryLeaseWentBack()
    => Assert.That(NativeRuntimeFactory.State,
                   Is.EqualTo("Absent"),
                   "the test left no lease behind");

  [Test]
  public void AGeneratedArmoniKStubAnswersOverThisInvoker()
  {
    using var channel = NativeRuntimeFactory.Channel(Endpoint);

    var results = new Results.ResultsClient(channel);

    Assert.That(() => results.GetServiceConfiguration(new Empty()),
                Throws.Nothing);
  }

  [Test]
  public async Task TheSameStubAnswersAsynchronously()
  {
    using var channel = NativeRuntimeFactory.Channel(Endpoint);

    var configuration = await new Results.ResultsClient(channel).GetServiceConfigurationAsync(new Empty())
                                                                .ConfigureAwait(false);

    Assert.That(configuration.DataChunkMaxSize,
                Is.GreaterThan(0),
                "the answer came from the service and not from a default");
  }
}
