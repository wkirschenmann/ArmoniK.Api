using System;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>What the tests call: an echo, a refusal, and a method that never answers.</summary>
public class EchoService : Echo.EchoBase
{
  public override async Task<EchoReply> Say(EchoRequest request,
                                            ServerCallContext context)
  {
    await context.WriteResponseHeadersAsync(new Metadata
                                            {
                                              {
                                                "x-answered", "yes"
                                              },
                                            })
                 .ConfigureAwait(false);

    return new EchoReply
           {
             Text        = request.Text,
             SawMetadata = Saw(context.RequestHeaders),
           };
  }

  public override Task<EchoReply> Refuse(EchoRequest request,
                                         ServerCallContext context)
  {
    var trailers = new Metadata
                   {
                     {
                       "x-reason", "policy"
                     },
                   };
    throw new RpcException(new Status(StatusCode.PermissionDenied,
                                      "not for you"),
                           trailers);
  }

  public override async Task<EchoReply> Never(EchoRequest request,
                                              ServerCallContext context)
  {
    await Task.Delay(Timeout.Infinite,
                     context.CancellationToken)
              .ConfigureAwait(false);
    return new EchoReply();
  }

  /// <summary>What the request carried, so a test can see it crossed rather than assume it.</summary>
  private static string Saw(Metadata headers)
  {
    var binary = headers.GetValueBytes("x-trace-bin");
    if (binary is not null)
    {
      return BitConverter.ToString(binary)
                         .Replace("-",
                                  string.Empty)
                         .ToLowerInvariant();
    }

    return headers.GetValue("x-request") ?? string.Empty;
  }
}
