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

#if NET8_0_OR_GREATER
using System;
using System.IO;
using System.Reflection;
using System.Runtime.Loader;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The netstandard2.0 build of the binding, run on a .NET runtime.</summary>
///
/// A .NET Framework consumer resolves to that build, and so does any other runtime that finds no
/// build for its own framework. The test host references the net8.0 or later build, so the
/// netstandard2.0 assembly, which the test project's build copies beside the host, is loaded
/// into a context of its own, where it is a different assembly from the one the host references.
public class NetStandardBindingTests
{
  private static readonly string BindingPath = Path.Combine(AppContext.BaseDirectory,
                                                          "netstandard2.0",
                                                          "ArmoniK.Api.Client.RustGrpcChannel.dll");

  private AssemblyLoadContext? context_;

  private Assembly binding_ = null!;

  [SetUp]
  public void LoadTheNetStandardBuild()
  {
    Assert.That(File.Exists(BindingPath),
                Is.True,
                $"the test project's build copies the netstandard2.0 binding to {BindingPath}");
    context_ = new AssemblyLoadContext("netstandard2.0 binding",
                                       true);
    binding_ = context_.LoadFromAssemblyPath(BindingPath);
  }

  [TearDown]
  public void Unload()
    => context_?.Unload();

  [Test]
  public void ItIsTheNetStandardBuildThatIsLoaded()
  {
    var framework = binding_.GetCustomAttribute<System.Runtime.Versioning.TargetFrameworkAttribute>();

    Assert.That(framework?.FrameworkName,
                Does.StartWith(".NETStandard,Version=v2.0"));
    Assert.That(binding_,
                Is.Not.SameAs(typeof(NativeLibrarySelection).Assembly));
  }

  [Test]
  public void TheMetricsBuildIsRefusedOnADotNetRuntimeWithAnErrorThatSaysWhyAndWhatToDo()
  {
    var selection = binding_.GetType("ArmoniK.Api.Client.RustGrpcChannel.NativeLibrarySelection")!;
    var builds = binding_.GetType("ArmoniK.Api.Client.RustGrpcChannel.NativeEngineBuild")!;
    var select = selection.GetMethod("Select")!;

    var refused = Assert.Throws<TargetInvocationException>(() => select.Invoke(null,
                                                                                new[]
                                                                                {
                                                                                  Enum.Parse(builds,
                                                                                             "Metrics"),
                                                                                }));

    Assert.That(refused!.InnerException,
                Is.TypeOf<PlatformNotSupportedException>());
    Assert.Multiple(() =>
                    {
                      var message = refused.InnerException!.Message;
                      Assert.That(message,
                                  Does.Contain("netstandard2.0"));
                      Assert.That(message,
                                  Does.Contain("default build would load"),
                                  "it says why");
                      Assert.That(message,
                                  Does.Contain("Run on .NET Framework"),
                                  "it says what to do");
                      Assert.That(message,
                                  Does.Contain("net8.0 or later"),
                                  "it names the other way");
                    });
    Assert.DoesNotThrow(() => select.Invoke(null,
                                            new[]
                                            {
                                              Enum.Parse(builds,
                                                         "Default"),
                                            }),
                        "the refusal leaves the choice open, and nothing answers with the default build");
  }

  [Test]
  public void TheDefaultBuildIsAcceptedOnADotNetRuntime()
  {
    var selection = binding_.GetType("ArmoniK.Api.Client.RustGrpcChannel.NativeLibrarySelection")!;
    var builds = binding_.GetType("ArmoniK.Api.Client.RustGrpcChannel.NativeEngineBuild")!;

    Assert.DoesNotThrow(() => selection.GetMethod("Select")!.Invoke(null,
                                                                    new[]
                                                                    {
                                                                      Enum.Parse(builds,
                                                                                 "Default"),
                                                                    }));
  }
}
#endif
