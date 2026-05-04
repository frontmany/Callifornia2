#include <cstdlib>
#include <iostream>
#include <memory>
#include <string>

#include <grpcpp/ext/proto_server_reflection_plugin.h>
#include <grpcpp/grpcpp.h>
#include <grpcpp/health_check_service_interface.h>

#include "absl/log/initialize.h"
#include "event_router.h"
#include "sfu_runtime.h"
#include "sfu_service_impl.h"

#include <rtc/rtc.hpp>

std::string listenAddrFromEnv() {
    if (const char* e = std::getenv("SFU_GRPC_ADDR")) {
        return std::string(e);
    }
    return "0.0.0.0:50051";
}

void runServer(const std::string& listenAddr) {
    auto eventRouter = std::make_shared<EventRouter>();
    auto runtime = std::make_shared<SfuRuntime>(eventRouter);
    SfuServiceImpl service(runtime);

    grpc::EnableDefaultHealthCheckService(true);
    grpc::reflection::InitProtoReflectionServerBuilderPlugin();

    grpc::ServerBuilder builder;
    builder.AddListeningPort(listenAddr, grpc::InsecureServerCredentials());
    builder.RegisterService(&service);

    std::unique_ptr<grpc::Server> server(builder.BuildAndStart());

    if (!server) {
        std::cerr << "BuildAndStart failed for " << listenAddr << "\n";
        std::exit(1);
    }

    std::cerr << "SFU gRPC listening on " << listenAddr << "\n";
    server->Wait();
}

int main(int argc, char* argv[]) {
    (void)argc;
    (void)argv;
    absl::InitializeLog();

    rtc::Preload();
    rtc::InitLogger(rtc::LogLevel::Warning);

    runServer(listenAddrFromEnv());
    return 0;
}
