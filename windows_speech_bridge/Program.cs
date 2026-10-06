// Experimental on-device Microsoft.Windows.AI.Speech bridge. Build and launch with MSIX identity.
using System.Text;
using System.Runtime.InteropServices;
using Microsoft.Windows.AI;
using Microsoft.Windows.AI.Speech;

Console.OutputEncoding = new UTF8Encoding(false);
Console.InputEncoding = new UTF8Encoding(false);

try
{
    PackageIdentity.EnsurePresent();
    if (args.Length == 1 && args[0] == "prepare")
    {
        // The caller shows an explicit model-download consent prompt before this command.
        if (SpeechRecognitionModel.GetReadyState() != AIFeatureReadyState.Ready)
        {
            await SpeechRecognitionModel.EnsureReadyAsync();
        }
        if (SpeechRecognitionModel.GetReadyState() != AIFeatureReadyState.Ready)
        {
            throw new InvalidOperationException("Windows 语音模型尚未就绪，请检查 Windows Update 与 AI 组件。");
        }
        Console.WriteLine("ready");
        return 0;
    }

    if (args.Length != 2 || args[0] != "transcribe" || !File.Exists(args[1]))
    {
        Console.Error.WriteLine("用法：to_words_windows_speech.exe prepare | transcribe <16kHz PCM WAV 路径>");
        return 2;
    }
    if (SpeechRecognitionModel.GetReadyState() != AIFeatureReadyState.Ready)
    {
        throw new InvalidOperationException("Windows 语音模型尚未安装，请先在 to_words 设置中点击“准备 Windows 语音模型”。");
    }
    var result = await SpeechRecognitionModel.TryCreateAsync();
    if (result.SpeechModel is null)
    {
        throw new InvalidOperationException($"创建 Windows 语音模型失败：{result.ExtendedError}");
    }
    using var model = result.SpeechModel;
    using var recognition = new BatchRecognition(model);
    Console.WriteLine((await recognition.RecognizeFromFile(args[1])).Trim());
    return 0;
}
catch (Exception error)
{
    Console.Error.WriteLine(error);
    return 1;
}

internal static class PackageIdentity
{
    [DllImport("kernel32.dll", ExactSpelling = true)]
    private static extern int GetCurrentPackageFullName(ref uint length, IntPtr fullName);

    internal static void EnsurePresent()
    {
        uint length = 0;
        int result = GetCurrentPackageFullName(ref length, IntPtr.Zero);
        if (result == 122) return; // ERROR_INSUFFICIENT_BUFFER: the process has package identity.
        if (result == 15700) // APPMODEL_ERROR_NO_PACKAGE
        {
            throw new InvalidOperationException("此程序尚未获得 MSIX 包身份，请注册签名后的身份包并从对应发布目录启动。");
        }
        throw new InvalidOperationException($"检查 MSIX 包身份失败：Windows 错误 {result}");
    }
}
