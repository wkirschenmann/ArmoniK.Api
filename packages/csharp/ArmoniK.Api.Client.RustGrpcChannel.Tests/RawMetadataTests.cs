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


using System.Text;

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The blob the engine reads, sized in one pass and written in another.</summary>
[TestFixture]
public class RawMetadataTests
{
  [Test]
  public void EveryEntryComesBackAsItWentIn()
  {
    var sent = new Metadata
               {
                 {
                   "x-plain", "value"
                 },
                 {
                   "x-empty", string.Empty
                 },
                 {
                   "x-bin", new byte[]
                            {
                              0, 1, 2, 255,
                            }
                 },
                 {
                   "x-empty-bin", new byte[0]
                 },
                 {
                   "x-repeated", "first"
                 },
                 {
                   "x-repeated", "second"
                 },
               };

    var read = RawMetadata.Decode(RawMetadata.Encode(sent));

    Assert.That(read.Count,
                Is.EqualTo(sent.Count));
    for (var index = 0; index < sent.Count; index++)
    {
      Assert.That(read[index]
                    .Key,
                  Is.EqualTo(sent[index]
                               .Key));
      Assert.That(read[index]
                    .IsBinary,
                  Is.EqualTo(sent[index]
                               .IsBinary),
                  sent[index]
                    .Key);

      if (sent[index]
          .IsBinary)
      {
        Assert.That(read[index]
                      .ValueBytes,
                    Is.EqualTo(sent[index]
                                 .ValueBytes));
      }
      else
      {
        Assert.That(read[index]
                      .Value,
                    Is.EqualTo(sent[index]
                                 .Value));
      }
    }
  }

  /// <summary>The size pass measures in bytes and so does the write pass.</summary>
  ///
  /// A text value sized in characters fits every value gRPC itself admits, because those are
  /// printable ASCII, and is short by exactly the difference for anything else. The codec is not
  /// the place that rule is enforced, so it is asserted here that it does not depend on it.
  [Test]
  public void ATextValueOfMoreBytesThanCharactersStillFits()
  {
    // Written as escapes so this file stays ASCII: three characters, six bytes once encoded.
    const string wide = "\u00e9\u00e8\u00ea";
    Assert.That(Encoding.UTF8.GetByteCount(wide),
                Is.EqualTo(2 * wide.Length),
                "the value has to be one this could get wrong");

    var sent = new Metadata
               {
                 {
                   "x-wide", wide
                 },
               };

    var read = RawMetadata.Decode(RawMetadata.Encode(sent));

    Assert.That(read[0]
                  .Value,
                Is.EqualTo(wide));
  }

  [Test]
  public void NothingToSayIsNoBytes()
    => Assert.That(RawMetadata.Encode(null),
                   Is.Empty);
}
