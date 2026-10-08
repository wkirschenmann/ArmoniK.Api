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

namespace System.Runtime.CompilerServices;

// The accessor of an `init` property carries a modifier that names this type, and a method is
// found by its signature, modifiers included. A library built against the netstandard2.0 target
// of this binding runs against the net8.0 one, so every target declares the type here and none
// takes the platform's, which is another type: the accessors then read the same on every target.
internal static class IsExternalInit
{
}
