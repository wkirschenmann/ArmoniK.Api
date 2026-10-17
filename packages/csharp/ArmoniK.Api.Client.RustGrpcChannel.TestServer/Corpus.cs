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

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>Compressible text, the same on every run and in both processes.</summary>
public static class Corpus
{
  private const int Words = 2048;

  /// <summary>Fills <paramref name="buffer" /> with words of 2 to 11 letters drawn from a fixed
  /// vocabulary, separated by spaces: deflate and zstd shrink it both by matches and by entropy
  /// coding.</summary>
  public static void Fill(byte[] buffer)
  {
    var state = 0x9E3779B97F4A7C15UL;

    ulong Next()
    {
      state ^= state << 13;
      state ^= state >> 7;
      state ^= state << 17;
      return state;
    }

    var vocabulary = new byte[Words][];
    for (var word = 0; word < Words; word++)
    {
      var letters = new byte[2 + (int)(Next() % 10)];
      for (var at = 0; at < letters.Length; at++)
      {
        letters[at] = (byte)('a' + (int)(Next() % 26));
      }

      vocabulary[word] = letters;
    }

    var written = 0;
    while (written < buffer.Length)
    {
      foreach (var letter in vocabulary[(int)(Next() % Words)])
      {
        if (written == buffer.Length)
        {
          return;
        }

        buffer[written++] = letter;
      }

      if (written < buffer.Length)
      {
        buffer[written++] = (byte)' ';
      }
    }
  }
}
