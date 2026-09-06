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

// The tests read the states the binding keeps and the name of the library it loads. Published
// instead, those would be a public surface nothing else calls, and three of them would have to
// launder an internal enum through a string to be expressible at all.
[assembly: InternalsVisibleTo("ArmoniK.Api.Client.RustGrpcChannel.Tests, PublicKey=0024000004800000940000000602000000240000525341310004000001000100b9cbe494cb23f1c9a351b8d0f211ba3f27afd44f1e683f1c08077b08372ad2649a9427e888c2aad68f010776c168f7a755e6ec591e48fcdd6928d2d6f1aeba06f7c3857437a5a15c7407756e17c3e1877a92eb5f9c82369731520f257bbca1f61a4caaa8aafc7aa40c5810cb81f16c68b4d4f8aa3044b09f7b417ca553bd53be")]
