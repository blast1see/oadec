# Dolby tool CLIs on the development machine (captured 2026-09-06)

## `dee.exe --help`

```
dee.exe, Version 5.2.1, Jun 10 2022
Options:
  --help, -h                               : Print this help.
  --xml, -x FILE                           : Specify a file with XML profile.
  --json, -j FILE                          : Specify a file with JSON profile.
  --schema, -s FILE                        : Specify a file to write XML schema.
  --input-audio, -a FILE                   : Specify a file with audio source. Not required if already provided in XML profile.
  --input-video, -v FILE                   : Specify a file with video source. Not required if already provided in XML profile.
  --output, -o FILE                        : Specify a file to write the output. Not required if already provided in XML profile.
  --log-file FILE                          : Specify a log file.
  --stdout                                 : If a log file is specified this option duplicates the log to standard output. Otherwise this option is ignored.
  --verbose LEVEL                          : LEVEL values are: quiet, normal, info, debug (default)
  --license-file, -l FILE                  : Specify a license file.
  --trace-file FILE                        : Specify a trace file.
  --trace-length NUMBER                    : Number of frames to keep in trace buffer (default=100000). 
  --progress                               : Enable progress bar. Ignored when no log file specified.
  --progress-interval MILLISECONDS         : Set progress reporting interval (default=1000).
  --diagnostics-interval MILLISECONDS      : Set diagnostics (e.g. MEM/CPU usage) reporting interval (default=1000).
  --add-elem                               : Add (overwrite if exists) XML element.
  --disable-xml-validation                 : Disable xml validation.
  --print-stages                           : Print available stages. Those not covered by license will be marked as disabled.
  --temp DIR                               : Set temp_dir to specified directory. This option overrides temp_dir specified in XML profile.
  --ignore-logs STRING                     : Ignore logs containing specified string or at least one of strings from comma-sperated list.

Usage Examples:
1. Write XML Schema to a file:
   dee.exe --schema C:\Users\CurrentUser\schema.xsd
2. Launch encoding job with progress reporting:
   dee.exe --progress --log-file log.log --xml ..\mxf_dv_mezz_to_dv_profile_5_hevc.xml
3. Launch encoding job with input from the CLI (overwrite input provided in XML if exists):
   dee.exe --xml ..\mxf_dv_mezz_to_dv_profile_5_hevc.xml --input-video C:\Video\Movie\movie.mxf
   dee.exe --xml ..\damf_atmos_mezz_to_atmos_ddp_ec3.xml --input-audio ..\Audio\printmaster.atmos


Time elapsed: 0.501143 seconds
```

## `atmos_info.exe --help`

```
AtmosInfo Tool (version 1.1)
Options:
	--input, -i FILE        : specify input atmos file
	--skip-validation, -s   : skip validation step for ADM BWF input
	--help, -h              : print help
	--version, -v           : print application and library version
```

## `bwf_info.exe --help`

```

Broadcast Wave Format Info.
This tool belongs to the Dolby Encoding Engine version 5.2.1
Interface version: 0.9.0 (Jun 10 2022):
  -h [ --help ]          Show this help.
  --loglevel arg (=info) Logging level followed by log customization options. Use "--morehelp 
                         loglevel" for more details. Values: debug|error|info|quiet|warning.
  -i [ --input ] arg     Input BWF file.
  --morehelp arg         Show more help about selected topic. Values: loglevel.


Execution time: 0.0005419 seconds
Exit code: 0
```

## `dee_dthd_encoder.exe Files/Dolby Media Encoder/resources/dee/dee_dthd_encoder.exe --help`

```

too many positional options have been specified on the command line
Dolby TrueHD with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.0.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --presentation arg                    Dolby TrueHD presentation configuration. Values: 
                                        2ch|6ch|8ch|atmos. Use "--morehelp presentation" for more 
                                        details.
  -f [ --input-format ] arg             Presentation's input format. Positional argument for option
                                        "presentation". Use "--morehelp input-format" for more 
                                        details. Values: atmos_mezz|wav|wav_list.
  -i [ --input ] arg                    Input file for presentation. Positional argument for option
                                        "presentation".
  -o [ --output ] arg                   Output file.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample".
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file".
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only|skip.
  --optimize-data-rate arg (=0)         Enables additional preprocessing that allows reduction of 
                                        peak data rate.
  --embed-timecode arg (=off)           Start timecode to be embedded into bitstream, to be used by
                                        authoring tools. Use "--morehelp embed-timecode" for more 
                                        details.
  --embedded-timecode-format arg (=auto)
                                        Frame rate of the embedded timecode. Value 'auto' uses 
                                        value passed with timecode-frame-rate switch. Values: 
                                        23.976|24|25|29.97|29.97df|30|50|59.94|60|auto.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        add-silence|embed-timecode|examples|extras|input-format|lou
                                        dness-management|presentation|loglevel|all.
  --temp-dir arg (=%USERPROFILE%\Documents\oadec\docs)
                                        Directory to store temporary files.
  --keep-temp arg (=0)                  Keep temporary files after execution. Values: 0|1.


ERROR: Failed to parse program options.
Execution time: 0.0033629 seconds
Exit code: 1
```

## `dee_dthd_encoder.exe Files/Dolby Media Encoder/resources/dee/dee_dthd_encoder.exe --morehelp input-format`

```

too many positional options have been specified on the command line
Dolby TrueHD with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.0.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --presentation arg                    Dolby TrueHD presentation configuration. Values: 
                                        2ch|6ch|8ch|atmos. Use "--morehelp presentation" for more 
                                        details.
  -f [ --input-format ] arg             Presentation's input format. Positional argument for option
                                        "presentation". Use "--morehelp input-format" for more 
                                        details. Values: atmos_mezz|wav|wav_list.
  -i [ --input ] arg                    Input file for presentation. Positional argument for option
                                        "presentation".
  -o [ --output ] arg                   Output file.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample".
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file".
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only|skip.
  --optimize-data-rate arg (=0)         Enables additional preprocessing that allows reduction of 
                                        peak data rate.
  --embed-timecode arg (=off)           Start timecode to be embedded into bitstream, to be used by
                                        authoring tools. Use "--morehelp embed-timecode" for more 
                                        details.
  --embedded-timecode-format arg (=auto)
                                        Frame rate of the embedded timecode. Value 'auto' uses 
                                        value passed with timecode-frame-rate switch. Values: 
                                        23.976|24|25|29.97|29.97df|30|50|59.94|60|auto.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        add-silence|embed-timecode|examples|extras|input-format|lou
                                        dness-management|presentation|loglevel|all.
  --temp-dir arg (=%USERPROFILE%\Documents\oadec\docs)
                                        Directory to store temporary files.
  --keep-temp arg (=0)                  Keep temporary files after execution. Values: 0|1.


ERROR: Failed to parse program options.
Execution time: 0.0004711 seconds
Exit code: 1
```

## `dee_dthd_encoder.exe Files/Dolby Media Encoder/resources/dee/dee_dthd_encoder.exe --morehelp presentation`

```

too many positional options have been specified on the command line
Dolby TrueHD with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.0.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --presentation arg                    Dolby TrueHD presentation configuration. Values: 
                                        2ch|6ch|8ch|atmos. Use "--morehelp presentation" for more 
                                        details.
  -f [ --input-format ] arg             Presentation's input format. Positional argument for option
                                        "presentation". Use "--morehelp input-format" for more 
                                        details. Values: atmos_mezz|wav|wav_list.
  -i [ --input ] arg                    Input file for presentation. Positional argument for option
                                        "presentation".
  -o [ --output ] arg                   Output file.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample".
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file".
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only|skip.
  --optimize-data-rate arg (=0)         Enables additional preprocessing that allows reduction of 
                                        peak data rate.
  --embed-timecode arg (=off)           Start timecode to be embedded into bitstream, to be used by
                                        authoring tools. Use "--morehelp embed-timecode" for more 
                                        details.
  --embedded-timecode-format arg (=auto)
                                        Frame rate of the embedded timecode. Value 'auto' uses 
                                        value passed with timecode-frame-rate switch. Values: 
                                        23.976|24|25|29.97|29.97df|30|50|59.94|60|auto.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        add-silence|embed-timecode|examples|extras|input-format|lou
                                        dness-management|presentation|loglevel|all.
  --temp-dir arg (=%USERPROFILE%\Documents\oadec\docs)
                                        Directory to store temporary files.
  --keep-temp arg (=0)                  Keep temporary files after execution. Values: 0|1.


ERROR: Failed to parse program options.
Execution time: 0.0004748 seconds
Exit code: 1
```

## `dee_ddpjoc_encoder.exe Files/Dolby Media Encoder/resources/dee/dee_ddpjoc_encoder.exe --help`

```

too many positional options have been specified on the command line
Dolby Digital Plus with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.1.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --cc arg (=1)                         Enable concurrent processing. Use "--morehelp cc" for more 
                                        details. Values: 0|1.
  --input-format arg                    Input format followed by format-specific options. Use 
                                        "--morehelp input-format" for more details. Values: 
                                        atmos_mezz|cbi_wav.
  -i [ --input ] arg                    Input file path. Can be provided multiple times. The order 
                                        of input parameters matters and is used as an encoding 
                                        order.
  -o [ --output ] arg                   Output bitstream file path. Can be provided multiple times.
                                        Output paths are matched to inputs based on their order of 
                                        appearance in command string.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'. Allowed for single 
                                        input only.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample". Allowed for single input only.
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file". Allowed 
                                        for single input only.
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details. In case of multiple outputs, silence will
                                        be prepended to the first file and/or appended to the last 
                                        one.
  --data-rate arg (=auto)               Target data rate in kbps. Values: 
                                        auto|384|448|576|640|768|1024.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only.
  -e [ --encoder ] arg                  Encoder configuration. Use "--morehelp encoder" for more 
                                        details.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        cc|add-silence|encoder|examples|extras|input-format|loudnes
                                        s-management|loglevel|all.


ERROR: Failed to parse program options.
Execution time: 0.0021318 seconds
Exit code: 1
```

## `dee_ddpjoc_encoder.exe Files/Dolby Media Encoder/resources/dee/dee_ddpjoc_encoder.exe --morehelp input-format`

```

too many positional options have been specified on the command line
Dolby Digital Plus with Dolby Atmos encoder
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 1.1.0 (Oct 17 2024):
  -h [ --help ]                         Show this help.
  -l [ --license ] arg (=C:\Program Files\Dolby Media Encoder\resources\dee\license.lic)
                                        License file.
  --loglevel arg (=info)                Logging level followed by log customization options. Use 
                                        "--morehelp loglevel" for more details. Values: 
                                        debug|error|info|quiet|warning.
  --overwrite arg (=0)                  Allow overwriting existing files. Values: 0|1.
  --progress arg (=0)                   Show progress in percentage. Values: 0|1.
  --cc arg (=1)                         Enable concurrent processing. Use "--morehelp cc" for more 
                                        details. Values: 0|1.
  --input-format arg                    Input format followed by format-specific options. Use 
                                        "--morehelp input-format" for more details. Values: 
                                        atmos_mezz|cbi_wav.
  -i [ --input ] arg                    Input file path. Can be provided multiple times. The order 
                                        of input parameters matters and is used as an encoding 
                                        order.
  -o [ --output ] arg                   Output bitstream file path. Can be provided multiple times.
                                        Output paths are matched to inputs based on their order of 
                                        appearance in command string.
  --start arg (=first_frame_of_action)  Start time given as a timecode, seconds (decimal number) or
                                        the keyword 'first_frame_of_action'. Allowed for single 
                                        input only.
  --duration arg (=-1)                  Duration in a timecode or seconds. Value '-1' means "up to 
                                        the last sample". Allowed for single input only.
  --end arg (=-1)                       End time given as a timecode or seconds (decimal number). 
                                        Value '-1' means "process till the end of file". Allowed 
                                        for single input only.
  --timecode-frame-rate arg (=auto)     Frame rate associated with the specified timecode. Values: 
                                        23.976|24|25|29.97|29.97df|30|48|50|59.94|60|not_indicated|
                                        auto.
  --time-base arg (=file_position)      Specify how the start/end/duration values are interpreted. 
                                        Values: file_position|embedded_timecode.
  --add-silence arg (=0:0)              Duration of silence to prepend and/or append to the output 
                                        in a format 'prepend_value':'append_value', expressed in 
                                        seconds or number of frames. Use "--morehelp add-silence" 
                                        for more details. In case of multiple outputs, silence will
                                        be prepended to the first file and/or appended to the last 
                                        one.
  --data-rate arg (=auto)               Target data rate in kbps. Values: 
                                        auto|384|448|576|640|768|1024.
  --loudness-management arg (=measure_only)
                                        Loudness management options. Syntax:
                                        "mode:option1=value1:option2=value2".
                                        Use "--morehelp loudness-management" for more details. 
                                        Values: measure_only.
  -e [ --encoder ] arg                  Encoder configuration. Use "--morehelp encoder" for more 
                                        details.
  --extras arg                          Configure advanced features. Use "--morehelp extras" for 
                                        more details.
  --morehelp arg                        Show more help about selected topic. Values: 
                                        cc|add-silence|encoder|examples|extras|input-format|loudnes
                                        s-management|loglevel|all.


ERROR: Failed to parse program options.
Execution time: 0.0004617 seconds
Exit code: 1
```

## `atmos_info.exe Files/Dolby Media Encoder/resources/dee/atmos_info.exe --help`

```

too many positional options have been specified on the command line
Atmos Info
This tool belongs to the Dolby Encoding Engine version 5.7.2.aec4fa9c-master
Interface version: 2.0.0 (Oct 17 2024):
  -h [ --help ]             Show this help.
  --loglevel arg (=info)    Logging level followed by log customization options. Use "--morehelp 
                            loglevel" for more details. Values: debug|error|info|quiet|warning.
  -i [ --input ] arg        Input Dolby Atmos mezzanine file.
  --validate arg (=1)       Validate ADM BWF input.
  -v [ --version ] arg (=0) Show version information.


ERROR: Failed to parse program options.
Execution time: 0.0002175 seconds
Exit code: 1
```

## `drp.exe Files/Dolby/Dolby Reference Player/drp.exe --help`

```

Dolby Reference Player 3.2.0.8040

Usage:
  drp [options] file

Arguments:
  file                                   File to be played

Options:
  -?, -h, --help                         Display the help text
  -v, --version                          Display version information
  --list-devices                         List available audio devices
  --verbose                              Enable debug prints
  --print-info                           Print bitstream info
  --device <value>                       Select output device from a devices list
                                           Possible values are:
                                           1               Kulaklýklar (AirPods Pro) (2 channels)
                                           2               Realtek Digital Output (Realtek(R) Audio) (2 channels)
                                           3               VG27AQML1A (NVIDIA High Definition Audio) (2 channels)
  --audio-out-file <value>               Specify the audio output file path to save raw audio samples as the WAVE format
  --video-out-file <value>               Specify the video output file path to save raw video samples as the YUV420 format
  --metadata-directory <value>           Dump metadata to CSV files
  --volume <value>                       Set volume, 1.0 = 100%
                                           Default: 1
                                           Range: 0 - 10
  --out-ch-config <value>                Set output channel configuration
                                           Default: 2.0
                                           Possible values are:
                                           2.0             channels: L,R
                                           3.1             channels: L,R,C,LFE
                                           5.1             channels: L,R,C,LFE,Ls,Rs
                                           7.1             channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs
                                           9.1             channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw
                                           5.1.2           channels: L,R,C,LFE,Ls,Rs,Ltm,Rtm
                                           5.1.4           channels: L,R,C,LFE,Ls,Rs,Ltf,Rtf,Ltr,Rtr
                                           7.1.2           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Ltm,Rtm
                                           7.1.4           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Ltf,Rtf,Ltr,Rtr
                                           7.1.6           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Ltf,Rtf,Ltm,Rtm,Ltr,Rtr
                                           9.1.2           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw,Ltm,Rtm
                                           9.1.4           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw,Ltf,Rtf,Ltr,Rtr
                                           9.1.6           channels: L,R,C,LFE,Ls,Rs,Lrs,Rrs,Lw,Rw,Ltf,Rtf,Ltm,Rtm,Ltr,Rtr
  --channel-map <value>                  Set mapping of decoded channels to output channels. The map is an array of pairs where
                                         first element in pair is a numerical value of channel id from GstAudioChannelPosition,
                                         and second element is an index of an output channel. For example, to switch stereo
                                         channels, set channel-map to "<<0,1>,<1,0>>"
  --ac3dec-dmx-mode <value>              Set two-channel downmix mode
                                           Default: auto
                                           Possible values are:
                                           auto            Auto detect
                                           lt/rt           Surround compatible
                                           lo/ro           Stereo
  --ac3dec-drc-boost <value>             Set the dynamic range control boost scale factor
                                           Default: 100
                                           Range: 0 - 100
  --ac3dec-drc-cut <value>               Set the dynamic range control cut scale factor
                                           Default: 100
                                           Range: 0 - 100
  --ac3dec-drc-mode <value>              Set the dynamic range control mode
                                           Default: line
                                           Possible values are:
                                           custom-0        Custom mode, analog dialnorm
                                           custom-1        Custom mode, digital dialnorm
                                           line            Line out mode
                                           rf              RF mode
                                           portable-8      Portable mode -8dB (output reference level is -8dB)
                                           portable-11     Portable mode -11dB (output reference level is -11dB)
                                           portable-14     Portable mode -14dB (output reference level is -14dB)
  --ac3dec-drc-suppress <value>          Suppress dynamic range control
                                           Default: false
                                           Possible values are: true, false
  --ac3dec-drop-delay <value>            Drop delay samples added by the decoder at the stream start
                                           Default: false
                                           Possible values are: true, false
  --ac4dec-ajoc-core-enabled <value>     Enable advanced joint object coding core decoder
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-dap-enabled <value>           Enable Dolby Audio Processing
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-de-level <value>              Set Dialog Enhancement preferred Level
                                           Default: 0
                                           Range: -12 - 12
  --ac4dec-dmx-mode <value>              Set stereo downmix mode
                                           Default: lo/ro
                                           Possible values are:
                                           lo/ro           Stereo
                                           lt/rt           Surround compatible, Pro Logic
                                           plII            Surround compatible, Pro Logic II
                                           hp              Headphone virtualization
                                           sp              Speaker virtualization
  --ac4dec-drc-enabled <value>           Enable dynamic range control
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-drop-delay <value>            Drop delay samples added by the decoder at the stream start
                                           Default: false
                                           Possible values are: true, false
  --ac4dec-front-speaker-angle <value>   Provide a front speaker angle
                                           Default: 10
                                           Range: 0 - 30
  --ac4dec-ieq-profile <value>           Select Intelligent Equalizer profile
                                           Default: disabled
                                           Possible values are:
                                           disabled        Disabled
                                           detailed        Detailed
                                           balanced        Balanced
                                           warm            Warm
  --ac4dec-ieq-strength <value>          Select Intelligent Equalizer strength
                                           Default: 10
                                           Range: 0 - 16
  --ac4dec-limiter-enabled <value>       Enable limiter
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-lpde-enabled <value>          Enable loudness-preserving mode in Dialogue Enancement
                                           Default: true
                                           Possible values are: true, false
  --ac4dec-main-assoc-mix-level <value>  Set main and associated mixing level
                                           Default: 0
                                           Range: -32 - 32
  --ac4dec-main-assoc-mode <value>       Set main and associated decoding modes
```

