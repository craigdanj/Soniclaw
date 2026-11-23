// Compare the portable classic WaveNet path against pinned upstream NAM.
#include "get_dsp.h"
#include <filesystem>
#include <fstream>
#include <vector>

int main(int argc, char** argv)
{
    if (argc != 4)
        return 2;
    auto engine = nam::get_dsp(std::filesystem::path(argv[1]), nam::DspLoadOptions{false});
    std::ifstream input(argv[2], std::ios::binary | std::ios::ate);
    const auto bytes = input.tellg();
    if (!input || bytes <= 0 || bytes % sizeof(float) != 0)
        return 3;
    std::vector<float> in(static_cast<std::size_t>(bytes) / sizeof(float));
    input.seekg(0);
    if (!input.read(reinterpret_cast<char*>(in.data()), bytes))
        return 4;
    std::vector<float> out(in.size());
    engine->SetPrewarmOnReset(false);
    engine->Reset(engine->GetExpectedSampleRate(), 128);
    for (std::size_t i = 0; i < in.size(); i += 128)
    {
        float* source = in.data() + i;
        float* dest = out.data() + i;
        engine->process(&source, &dest, static_cast<int>(std::min<std::size_t>(128, in.size() - i)));
    }
    std::ofstream output(argv[3], std::ios::binary);
    output.write(reinterpret_cast<const char*>(out.data()), bytes);
    return output ? 0 : 5;
}
