#pragma once

#include <chrono>
#include <grpcpp/grpcpp.h>
#include <memory>

#include "signaling.grpc.pb.h"

class SfuRuntime;

class SfuServiceImpl final : public sfu::SFUService::Service {
public:
    explicit SfuServiceImpl(std::shared_ptr<SfuRuntime> runtime);

    grpc::Status CreatePeer(grpc::ServerContext* context,
                            const sfu::CreatePeerRequest* request,
                            sfu::CreatePeerResponse* response) override;

    grpc::Status DeletePeer(grpc::ServerContext* context,
                            const sfu::DeletePeerRequest* request,
                            sfu::DeletePeerResponse* response) override;

    grpc::Status HandleSDP(grpc::ServerContext* context,
                           const sfu::HandleSDPRequest* request,
                           sfu::HandleSDPResponse* response) override;

    grpc::Status AddICECandidate(grpc::ServerContext* context,
                                 const sfu::AddICECandidateRequest* request,
                                 sfu::AddICECandidateResponse* response) override;

    grpc::Status SubscribeEvents(grpc::ServerContext* context,
                                 const sfu::SubscribeEventsRequest* request,
                                 grpc::ServerWriter<sfu::SFUEvent>* writer) override;

    grpc::Status Ping(grpc::ServerContext* context,
                      const sfu::PingRequest* request,
                      sfu::PingResponse* response) override;

private:
    std::shared_ptr<SfuRuntime> m_runtime;
    static constexpr std::chrono::seconds m_heartbeatInterval{15};
};
