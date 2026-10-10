// What the ask bar can turn a sentence into. One catalog per kind of device.
//
// Each entry:
//   command   what is sent; "{}" is filled with the argument found in the sentence
//   say       a few ways a person might ask for it, in any language. More
//             phrasings, and phrasings in the language you type, match better.
//   arg       RegExp for the argument (with "{}"). Without the language model
//             it picks the argument out of the sentence, and a command with
//             "{}" but no arg is left for the user to complete; with the model
//             it is what the model's value must look like (default: no spaces)
//   confirm   true for anything that changes the device: it is shown first and
//             only sent on a second Enter

export const CATALOGS = {
  "Linux shell": [
    { command: "ls", say: ["현재 폴더 파일 리스트", "파일 목록 보여줘", "list the files in this directory"] },
    { command: "ls -al", say: ["숨김 파일까지 자세히 목록", "파일 권한과 크기 보기", "list all files with details"] },
    { command: "ls {}", say: ["특정 폴더의 파일 목록", "list the files in a given directory"] },
    { command: "cat {}", say: ["파일 내용 보기", "print the contents of a file"] },
    { command: "tail -n 30 {}", say: ["파일 끝부분 보기", "로그 파일 마지막 줄들", "show the end of a file"] },
    { command: "ping -c 4 {}", say: ["핑 보내기", "주소가 응답하는지 확인", "ping a host"] },
    { command: "pwd", say: ["지금 어느 폴더에 있지", "현재 경로", "print the current directory"] },
    { command: "cd ..", say: ["상위 폴더로 이동", "한 단계 위로", "go up one directory"] },
    { command: "df -h", say: ["디스크 남은 용량", "저장 공간 얼마나 남았나", "free disk space"] },
    { command: "free -h", say: ["메모리 사용량", "램 얼마나 쓰고 있나", "memory usage"] },
    { command: "uptime", say: ["켜진 지 얼마나 됐나", "부팅 후 경과 시간", "how long has it been running"] },
    { command: "ps aux", say: ["실행 중인 프로세스 목록", "뭐가 돌고 있나", "list running processes"] },
    { command: "ip addr", say: ["IP 주소 확인", "네트워크 인터페이스 정보", "show ip addresses"] },
    { command: "dmesg | tail -n 30", say: ["커널 로그 마지막 부분", "최근 커널 메시지", "recent kernel messages"] },
    { command: "date", say: ["지금 시간", "현재 날짜와 시각", "current date and time"] },
    { command: "uname -a", say: ["커널 버전", "운영체제 정보", "kernel and system version"] },
    { command: "reboot", confirm: true, say: ["재부팅", "다시 시작해", "restart the machine"] },
  ],

  "Mock switch": [
    { command: "help", say: ["쓸 수 있는 명령 보여줘", "도움말", "what commands are there"] },
    { command: "show vlan", say: ["VLAN 상태 확인", "지금 VLAN 몇 번이야", "show the vlan"] },
    { command: "show version", say: ["펌웨어 버전", "소프트웨어 버전 확인", "firmware version"] },
    { command: "show running-config", say: ["현재 설정 전체 보기", "설정 내용 출력", "show the configuration"] },
    { command: "show log", say: ["로그 보여줘", "이벤트 기록 확인", "show the system log"] },
    {
      command: "set vlan {}",
      arg: /\d{1,4}/,
      confirm: true,
      say: ["VLAN 번호 바꿔", "VLAN을 100으로 설정", "change the vlan id"],
    },
    { command: "set hostname {}", confirm: true, say: ["장비 이름 바꿔", "호스트네임 변경", "rename the device"] },
    { command: "set mode trunk", confirm: true, say: ["트렁크 모드로 변경", "포트를 트렁크로", "switch to trunk mode"] },
    { command: "set mode access", confirm: true, say: ["액세스 모드로 변경", "포트를 액세스로", "switch to access mode"] },
    { command: "set enabled off", confirm: true, say: ["포트 꺼", "인터페이스 비활성화", "shut the port down"] },
    { command: "set enabled on", confirm: true, say: ["포트 켜", "인터페이스 활성화", "bring the port up"] },
  ],
};
