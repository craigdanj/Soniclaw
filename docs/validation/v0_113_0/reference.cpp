// Compare a generated fixture against the upstream NAM LSTM implementation.
#include <fstream>
#include <vector>
#include "lstm.h"

int main(int argc, char** argv)
{
    if (argc != 4)
        return 2;
    std::ifstream model_stream(argv[1]);
    if (!model_stream)
        return 3;
    nlohmann::json model = nlohmann::json::parse(model_stream);
    const auto shape = nam::lstm::parse_config_json(model.at("config"));
    auto weights = model.at("weights").get<std::vector<float>>();
    nam::lstm::LSTM engine(shape.in_channels, shape.out_channels, shape.num_layers,
                           shape.input_size, shape.hidden_size, weights,
                           model.at("sample_rate").get<double>());
    std::ifstream input(argv[2], std::ios::binary | std::ios::ate);
    const auto bytes = input.tellg();
    if (!input || bytes <= 0 || bytes % sizeof(float) != 0)
        return 4;
    std::vector<float> in(static_cast<std::size_t>(bytes) / sizeof(float));
    input.seekg(0);
    if (!input.read(reinterpret_cast<char*>(in.data()), bytes))
        return 5;
    std::vector<float> out(in.size());
    float* in_ch = in.data();
    float* out_ch = out.data();
    engine.process(&in_ch, &out_ch, static_cast<int>(in.size()));
    std::ofstream output(argv[3], std::ios::binary);
    output.write(reinterpret_cast<const char*>(out.data()), bytes);
    return output ? 0 : 6;
}

