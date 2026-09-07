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


using System.Runtime.CompilerServices;

// This tool's surface is its command line; the tests read the two steps behind it. Neither is
// published, so they are internal and the tests are named a friend rather than the whole thing
// being made public for their benefit. Unsigned, so the name alone identifies them.
[assembly: InternalsVisibleTo("ArmoniK.Api.Client.RustGrpcChannel.OptionsGenerator.Tests")]
